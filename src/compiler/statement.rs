//! mslang 语句编译（task 19）。
//!
//! 将 AST 语句节点翻译为字节码指令序列。语句编译的核心是控制流的
//! 跳转指令与循环的 patch 机制。
//!
//! 参照 [19-compile-statements](../../../docs/mslang/tasks/19-compile-statements.md)。

use crate::ast::node::{
    AssignOp, BinaryOp, ExceptClause, Expr, SelectCase, SelectOp, Stmt, UnaryOp,
};
use crate::vm::object::{alloc_function, alloc_string, Function};

use super::{CompilationUnit, Compiler, Local, OpCode};

// ---- 语句编译分发器 ----

impl Compiler {
    /// 语句编译入口。根据 Stmt 变体路由到对应编译方法。
    /// task 57：使用 stmt.line()（AST 携带的源码行号）覆盖传入的 line，使每条
    /// 语句的字节码记录真实行号（行号表 §1）。
    pub fn compile_statement(&mut self, stmt: &Stmt, _passed_line: usize) -> Result<(), String> {
        let line = stmt.line();
        match stmt {
            Stmt::VarDecl {
                name, initializer, ..
            }
            | Stmt::ShortVarDecl {
                name, initializer, ..
            } => self.compile_var_decl(name, initializer, false, line),
            Stmt::ConstDecl {
                name, initializer, ..
            } => self.compile_var_decl(name, initializer, true, line),
            Stmt::Assign {
                target, op, value, ..
            } => self.compile_assign_stmt(target, op, value, line),
            Stmt::ExprStmt { expr, .. } => self.compile_expr_stmt(expr, line),
            Stmt::Block { statements, .. } => self.compile_block(statements, line),
            Stmt::If {
                condition,
                then_block,
                elif_clauses,
                else_block,
                ..
            } => self.compile_if(condition, then_block, elif_clauses, else_block, line),
            Stmt::While {
                condition, body, ..
            } => self.compile_while(condition, body, line),
            Stmt::ForIn {
                variable,
                second_variable,
                iterable,
                body,
                ..
            } => self.compile_for_in(variable, second_variable.as_deref(), iterable, body, line),
            Stmt::Break { .. } => self.compile_break(line),
            Stmt::Continue { .. } => self.compile_continue(line),
            Stmt::Return { values, .. } => self.compile_return(values, line),
            Stmt::Nonlocal { names, .. } => self.compile_nonlocal(names, line),
            Stmt::Global { names, .. } => self.compile_global(names, line),
            Stmt::FnDecl {
                name,
                params,
                body,
                is_async,
                ..
            } => self.compile_fn_decl(name, params, body, *is_async, line),
            Stmt::ClassDecl {
                name,
                parent,
                methods,
                class_vars,
                ..
            } => self.compile_class_decl(name, parent, methods, class_vars, line),
            Stmt::Defer { expr, .. } => self.compile_defer(expr, line),
            Stmt::Try {
                try_block,
                except_clauses,
                finally_block,
                ..
            } => self.compile_try(try_block, except_clauses, finally_block, line),
            Stmt::With {
                expression,
                alias,
                body,
                ..
            } => self.compile_with(expression, alias, body, line),
            Stmt::Import {
                module_path,
                alias,
                is_stdlib,
                ..
            } => self.compile_import(module_path, alias, *is_stdlib, line),
            Stmt::FromImport {
                module_path,
                targets,
                is_stdlib,
                ..
            } => self.compile_from_import(module_path, targets, *is_stdlib, line),
            Stmt::Throw { expr, .. } => self.compile_throw(expr, line),
            Stmt::Decorated {
                decorators, target, ..
            } => self.compile_decorated(decorators, target, line),
            Stmt::Select {
                cases,
                default_block,
                ..
            } => self.compile_select(cases, default_block, line),
        }
    }
}

// ---- task 45：import / from...import 编译 ----

impl Compiler {
    /// 编码模块名为常量池字符串索引。dotted path 用 "." 连接；`@std` 折叠为 "@std:" 前缀
    ///（§3 前缀编码：无新 opcode，VM/搜索逻辑透明）。
    fn module_const_idx(&mut self, module_path: &[String], is_stdlib: bool) -> Result<u16, String> {
        let joined = module_path.join(".");
        let full = if is_stdlib {
            format!("@std:{}", joined)
        } else {
            joined
        };
        let idx = self.add_constant(alloc_string(&full));
        u16::try_from(idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())
    }

    /// 发射 IMPORT module_idx(2)，将 Module 对象压栈。
    fn emit_import(
        &mut self,
        module_path: &[String],
        is_stdlib: bool,
        line: usize,
    ) -> Result<(), String> {
        let idx = self.module_const_idx(module_path, is_stdlib)?;
        self.emit_byte(OpCode::Import as u8, line);
        self.emit_bytes(&idx.to_be_bytes(), line);
        Ok(())
    }

    /// 顶层（无父单元）→ STORE_GLOBAL；函数体内 → 声明局部 slot + STORE_LOCAL。
    /// import 绑定的名字须按作用域存入，使后续引用可解析。
    fn emit_import_binding(&mut self, name: &str, line: usize) -> Result<(), String> {
        if self.unit.parent.is_null() {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::StoreGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        } else {
            if self.resolve_local(name).is_none() {
                self.declare_local(name, line)?;
            }
            self.emit_store_name(name, line)?;
        }
        Ok(())
    }

    /// `import foo`        → IMPORT "foo"; STORE "foo"
    /// `import foo as bar` → IMPORT "foo"; STORE "bar"
    /// `import a.b.c`      → IMPORT "a.b.c"; STORE "a"（首段绑定；dotted 嵌套属性访问
    ///   需父包暴露子模块为属性，属后续扩展，本 MVP 绑定首段名）。
    fn compile_import(
        &mut self,
        module_path: &[String],
        alias: &Option<String>,
        is_stdlib: bool,
        line: usize,
    ) -> Result<(), String> {
        self.emit_import(module_path, is_stdlib, line)?;
        // 绑定名：alias 优先，否则首段（单段即模块名）。
        let bind_name = alias
            .clone()
            .or_else(|| module_path.first().cloned())
            .ok_or_else(|| "import: empty module path".to_string())?;
        self.emit_import_binding(&bind_name, line)
    }

    /// `from foo import a, b as c` → IMPORT "foo"; GET_ATTR "a"; STORE "a";
    ///   GET_ATTR "b"; STORE "c"。每个 target 取模块导出并按名/别名绑定。
    fn compile_from_import(
        &mut self,
        module_path: &[String],
        targets: &[(String, Option<String>)],
        is_stdlib: bool,
        line: usize,
    ) -> Result<(), String> {
        if targets.is_empty() {
            return Err("from...import: no targets".to_string());
        }
        for (name, alias) in targets {
            // 每个 target：压模块 → GET_ATTR name → STORE 绑定名。
            // （可优化为单次 IMPORT + 多次 GET_ATTR，但保持每 target 自包含便于绑定。）
            self.emit_import(module_path, is_stdlib, line)?;
            let attr_idx = self.add_constant(alloc_string(name));
            let attr_idx = u16::try_from(attr_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::GetAttr as u8, line);
            self.emit_bytes(&attr_idx.to_be_bytes(), line);
            let bind_name = alias.clone().unwrap_or_else(|| name.clone());
            self.emit_import_binding(&bind_name, line)?;
        }
        Ok(())
    }
}

// ---- 声明、赋值、表达式语句、block、return、nonlocal/global ----

impl Compiler {
    /// 编译 var/短声明/const 声明。三者均：求值右值 → 声明局部 → 存入 slot。
    /// `is_const` 为 true 时先做常量表达式校验。
    ///
    /// 解析器对 `name = value`（标识符目标 + 简单赋值）也产出 VarDecl，
    /// 与 `var`/`:=` 同走此方法。mslang 为函数级作用域，重复"声明"同名变量
    /// 视为更新现有 slot（仅首次创建新局部），匹配赋值语义。
    fn compile_var_decl(
        &mut self,
        name: &str,
        init: &Expr,
        is_const: bool,
        line: usize,
    ) -> Result<(), String> {
        if is_const {
            self.validate_const_expr(init, line)?;
        }
        self.compile_expression(init, line)?;
        // nonlocal 写语义（04-functions.md）：声明为 nonlocal 的名字强制走上值路径，
        // 不在当前作用域创建新局部（否则会遮蔽外层变量而非写入它）。
        if self.nonlocal_names.contains(name) {
            return match self.resolve_upvalue(name) {
                Some(idx) => {
                    self.emit_byte(OpCode::StoreUpvalue as u8, line);
                    self.emit_byte(idx as u8, line);
                    Ok(())
                }
                None => Err(format!("no binding for nonlocal '{}'", name)),
            };
        }
        // global 写语义（03-syntax.md § global 声明）：语句级 `=` 对 global 名字
        // 写全局，不在当前作用域创建新局部（parse 层把简单赋值解析为 VarDecl，
        // 无此检查会遮蔽全局）。
        if self.global_names.contains(name) {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow".to_string())?;
            self.emit_byte(OpCode::StoreGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
            return Ok(());
        }
        // task 45 §7：模块模式下顶层（无父单元）const/var/`=` 走 STORE_GLOBAL，
        // 使 execute_module 能经 globals 捕获模块顶层定义（与 fn/class 顶层一致）。
        // 函数体内（有父单元）不受影响，仍走局部 slot。
        if self.module_mode && self.unit.parent.is_null() {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::StoreGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
            return Ok(());
        }
        let slot = match self.resolve_local(name) {
            Some(slot) => slot,
            None => {
                self.declare_local(name, line)?;
                self.resolve_local(name)
                    .ok_or_else(|| format!("internal: local '{}' not found after declare", name))?
            }
        };
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(slot as u8, line);
        Ok(())
    }

    /// const 表达式语法形式校验：仅允许字面量、一元取反（`-`/`~`）、
    /// 二元算术与位运算、括号分组。完整常量折叠求值由后续 task 扩展。
    fn validate_const_expr(&self, expr: &Expr, line: usize) -> Result<(), String> {
        match expr {
            Expr::Literal(_) => Ok(()),
            Expr::Grouping { expr } => self.validate_const_expr(expr, line),
            Expr::Unary { op, operand } => match op {
                UnaryOp::Negate | UnaryOp::BitNot => self.validate_const_expr(operand, line),
                _ => Err(format!(
                    "line {}: const initializer contains unsupported unary op",
                    line
                )),
            },
            Expr::Binary { left, op, right } => {
                if matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::FloorDiv
                        | BinaryOp::Modulo
                        | BinaryOp::Power
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::LeftShift
                        | BinaryOp::RightShift
                ) {
                    self.validate_const_expr(left, line)?;
                    self.validate_const_expr(right, line)
                } else {
                    Err(format!(
                        "line {}: const initializer must be a constant expression",
                        line
                    ))
                }
            }
            _ => Err(format!(
                "line {}: const initializer must be a constant expression",
                line
            )),
        }
    }

    /// 编译赋值语句（含复合赋值与属性/下标目标）。
    /// 复合赋值、目标类型的运算与存储逻辑全部由 task 18 的 compile_assignment
    /// 统一实现，本方法仅做语句包装 + POP 丢弃结果值。
    fn compile_assign_stmt(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        value: &Expr,
        line: usize,
    ) -> Result<(), String> {
        let assign_expr = Expr::Assign {
            target: Box::new(target.clone()),
            op: *op,
            value: Box::new(value.clone()),
        };
        self.compile_expression(&assign_expr, line)?;
        self.emit_byte(OpCode::Pop as u8, line);
        Ok(())
    }

    /// 编译表达式语句：求值表达式 → 弹出结果。
    fn compile_expr_stmt(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
        self.compile_expression(expr, line)?;
        self.emit_byte(OpCode::Pop as u8, line);
        Ok(())
    }

    /// 编译 block 语句：顺序编译内部语句。
    ///
    /// `pub(super)`：除本模块外，task 29 的 `compile_fn_literal`（expression.rs）也需
    /// 编译匿名函数体（语句向量），与具名函数体编译路径一致。
    pub(super) fn compile_block(&mut self, stmts: &[Stmt], line: usize) -> Result<(), String> {
        for stmt in stmts {
            self.compile_statement(stmt, line)?;
        }
        Ok(())
    }

    /// 编译函数/方法体为 CLOSURE 指令（含逐上值操作数），闭包留在栈顶，**不**绑定名。
    /// 创建独立编译单元（parent 链接父单元以启用上值解析）。编译函数体后追加隐式
    /// `NIL + RETURN`。task 28 改造：存 **Function**（非 Closure）入常量池，发 `CLOSURE`
    /// 指令 + 逐上值操作数（运行期包装），并写真值 `upvalue_count`。
    /// 顶层函数（compile_fn_decl）后接 STORE_GLOBAL；类方法（compile_class_decl）后接 METHOD。
    ///
    /// slot 0 约定：
    ///   - 普通函数（is_method=false）：slot 0 = `<self>`（closure 占位），参数从 slot 1 起，
    ///     与 CALL 的 stack_base=callee_idx 自洽。
    ///   - 方法（is_method=true）：slot 0 = 首参数 `self`（由 BoundMethod CALL handler 写入），
    ///     其余参数从 slot 1 起。首参数必须为 `self`，否则编译期报错（task 41 §4）。
    fn compile_function_closure(
        &mut self,
        name: &str,
        params: &[crate::ast::node::Param],
        body: &[Stmt],
        line: usize,
        is_method: bool,
        is_async: bool,
    ) -> Result<(), String> {
        // task 41 §4：方法首参数必须为 self（self 在词法层为关键字，仅此位置可作标识符）。
        if is_method && (params.is_empty() || params[0].name != "self") {
            return Err(format!(
                "method '{}' must have 'self' as first parameter",
                name
            ));
        }
        let mut func_unit = CompilationUnit {
            chunk: super::Chunk::new(),
            // 普通函数：slot 0 预留给被调用者（closure 自身），参数从 slot 1 起。
            // 方法（task 41）：slot 0 = self（首参数），其余参数从 slot 1 起；
            //   由 CALL 的 BoundMethod handler 将 receiver 写入 slot 0。
            locals: if is_method {
                vec![]
            } else {
                vec![Local {
                    name: "<self>".to_string(),
                    depth: 0,
                    is_captured: false,
                }]
            },
            upvalues: Vec::new(),
            scope_depth: 0,
            is_generator: false,
            is_async_context: is_async, // async fn 体允许 await；普通 fn 不允许
            parent: std::ptr::null(),
        };
        // task 31：参数顺序校验（普通 → 默认 → 可变）+ 默认/可变参数分类。
        super::validate_param_order(params)?;
        let mut required_arity = 0usize;
        let mut default_values = Vec::new();
        let mut has_variadic = false;
        for param in params {
            if param.is_variadic {
                has_variadic = true;
            } else if param.default.is_some() {
                let val = super::eval_default(param.default.as_ref().unwrap())?;
                default_values.push(val);
            } else {
                required_arity += 1;
            }
            // 所有参数都注册为 local（含 variadic 和 default）。
            func_unit.locals.push(Local {
                name: param.name.clone(),
                depth: 0,
                is_captured: false,
            });
        }

        // 换出父单元，编译函数体。期间 func_unit.parent 指向 saved_unit（裸指针，
        // 规避 self-referential 借用冲突，见 CompilationUnit.parent 字段注释）。
        // nonlocal/global 声明集合按函数体隔离保存/恢复：声明仅在所属函数内生效，
        // 不泄漏到随后编译的兄弟函数。
        let saved_unit = std::mem::replace(&mut self.unit, func_unit);
        let saved_nonlocal = std::mem::take(&mut self.nonlocal_names);
        let saved_global = std::mem::take(&mut self.global_names);
        self.unit.parent = std::ptr::addr_of!(saved_unit);
        self.compile_block(body, line)?;
        self.emit_byte(OpCode::Nil as u8, line);
        self.emit_return(line);
        self.nonlocal_names = saved_nonlocal;
        self.global_names = saved_global;
        let func_unit = std::mem::replace(&mut self.unit, saved_unit);

        // 上值捕获回填：函数体中 is_local=true 的上值对应父单元的局部变量，
        // 标记其 is_captured（驱动 end_scope 发射 CLOSE_UPVALUE）。此时 self.unit
        // 已恢复为父单元，func_unit 为刚编译的子单元。先收集再写回以避开借用。
        let captured_locals: Vec<usize> = func_unit
            .upvalues
            .iter()
            .filter(|uv| uv.is_local)
            .map(|uv| uv.index)
            .collect();
        for idx in captured_locals {
            if idx < self.unit.locals.len() {
                self.unit.locals[idx].is_captured = true;
            }
        }

        // 存 Function（非 Closure）入常量池 —— CLOSURE 指令运行期包装。
        let function = Function {
            name: name.to_string(),
            // 固定参数总数（普通 + 默认，不含可变）。
            arity: params.iter().filter(|p| !p.is_variadic).count(),
            code: func_unit.chunk.code,
            constants: func_unit.chunk.constants,
            upvalue_count: func_unit.upvalues.len(),
            source_file: self.source_file.clone(),
            default_values,
            has_variadic,
            required_arity,
            is_generator: func_unit.is_generator,
            locals_count: func_unit.locals.len(),
            is_async,
            lines: func_unit.chunk.lines,
        };
        let func_idx = self.add_constant(alloc_function(function));
        let func_idx = u16::try_from(func_idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;

        // 发 CLOSURE(func_idx) + 逐上值操作数（is_local:1 + index:1 每上值）。
        self.emit_byte(OpCode::Closure as u8, line);
        self.emit_bytes(&func_idx.to_be_bytes(), line);
        for uv in &func_unit.upvalues {
            self.emit_byte(if uv.is_local { 1 } else { 0 }, line);
            let idx = u8::try_from(uv.index).map_err(|_| {
                format!(
                    "upvalue index {} exceeds 255 (function too large)",
                    uv.index
                )
            })?;
            self.emit_byte(idx, line);
        }

        Ok(())
    }

    /// 编译顶层函数声明（task 27/28）：编译闭包 + 绑定函数名。
    ///
    /// task 44：作用域感知。顶层（parent 为空）用 STORE_GLOBAL；函数体内声明的具名
    /// 函数声明局部 slot 并用 STORE_LOCAL，使装饰器与局部引用一致，不污染全局。
    fn compile_fn_decl(
        &mut self,
        name: &str,
        params: &[crate::ast::node::Param],
        body: &[Stmt],
        is_async: bool,
        line: usize,
    ) -> Result<(), String> {
        self.compile_function_closure(name, params, body, line, false, is_async)?;
        if self.unit.parent.is_null() {
            // 顶层：STORE_GLOBAL（与 task 27 一致）
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::StoreGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        } else {
            // 函数体内：声明局部 slot + STORE_LOCAL（不污染全局）
            if self.resolve_local(name).is_none() {
                self.declare_local(name, line)?;
            }
            self.emit_store_name(name, line)?;
        }
        Ok(())
    }

    /// task 44：编译装饰器语句。语义糖——等价于先定义 target，再将 target 名称
    /// 从内到外依次赋值为 `decorator(target)`。
    ///
    /// 编译策略（无 SWAP/ROT，栈每轮平衡）：
    /// 1. 编译 target（fn/class 语句）——值已绑定到对应变量（全局或局部）。
    /// 2. 反向遍历 decorators（靠近 target 的先应用）：
    ///    - 编译 decorator 表达式 → [decorator]
    ///    - emit_load_name(target) → [decorator, current]
    ///    - CALL 1 → [decorator(current)]
    ///    - emit_store_name(target) → 栈平衡
    fn compile_decorated(
        &mut self,
        decorators: &[Expr],
        target: &Stmt,
        line: usize,
    ) -> Result<(), String> {
        // 1. 编译目标（fn 或 class）。语句编译不留栈值，值已绑定到变量。
        self.compile_statement(target, line)?;

        if decorators.is_empty() {
            return Ok(());
        }

        // 2. 目标名（FnDecl/ClassDecl 的声明名）。
        let target_name = target.decl_name().ok_or_else(|| {
            "decorator target must be a function or class declaration".to_string()
        })?;

        // 3. 反向遍历（从内到外：靠近 target 的先应用）。
        for dec_expr in decorators.iter().rev() {
            self.compile_expression(dec_expr, line)?; // [decorator]
            self.emit_load_name(target_name, line)?; // [decorator, current]
            self.emit_byte(OpCode::Call as u8, line); // [decorator(current)]
            self.emit_byte(1, line);
            self.emit_store_name(target_name, line)?; // 变量 = 结果，栈平衡
        }

        Ok(())
    }

    /// task 44：按作用域加载变量名。顶层用 LOAD_GLOBAL，函数体内用 LOAD_LOCAL(slot)。
    fn emit_load_name(&mut self, name: &str, line: usize) -> Result<(), String> {
        if let Some(slot) = self.resolve_local(name) {
            let slot = u8::try_from(slot).map_err(|_| "local slot exceeds 255".to_string())?;
            self.emit_byte(OpCode::LoadLocal as u8, line);
            self.emit_byte(slot, line);
        } else {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::LoadGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        }
        Ok(())
    }

    /// task 44：按作用域存储变量名。顶层用 STORE_GLOBAL，函数体内用 STORE_LOCAL(slot)。
    /// 栈顶值弹出后写入目标。嵌套作用域的具名函数声明经此存为局部。
    fn emit_store_name(&mut self, name: &str, line: usize) -> Result<(), String> {
        if let Some(slot) = self.resolve_local(name) {
            let slot = u8::try_from(slot).map_err(|_| "local slot exceeds 255".to_string())?;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot, line);
        } else {
            let name_idx = self.add_constant(alloc_string(name));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::StoreGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
        }
        Ok(())
    }

    /// 编译 class 定义（task 40，task 42 扩展继承）。
    /// 字节码布局：
    ///   CLASS name            → 类对象压栈
    ///   [有父类] LOAD_GLOBAL parent; INHERIT  → 仍留 [class]
    ///   STORE_GLOBAL name     → 存为全局变量（先于属性/方法，使其可自引用）
    ///   [每个类属性] DUP class; <expr>; SET_ATTR name   → 仍留 [class]
    ///   [每个方法]   <CLOSURE>;  METHOD name            → 仍留 [class]
    /// 仅支持顶层 class 定义。
    fn compile_class_decl(
        &mut self,
        name: &str,
        parent: &Option<String>,
        methods: &[Stmt],
        class_vars: &[(String, Expr)],
        line: usize,
    ) -> Result<(), String> {
        // 函数内定义 class 暂不支持（task 17 局部变量规则未覆盖 class）。
        if !self.unit.parent.is_null() {
            return Err("class definition inside function not supported".into());
        }

        // CLASS name → [class]
        let name_idx = self.add_constant(alloc_string(name));
        let name_idx = u16::try_from(name_idx)
            .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
        self.emit_byte(OpCode::Class as u8, line);
        self.emit_bytes(&name_idx.to_be_bytes(), line);

        // task 42：有显式父类时，LOAD_GLOBAL parent; INHERIT（class 仍在栈顶）。
        // INHERIT 在 VM 端覆写 parent（CLASS 已默认链接 Object，见 VM CLASS handler）。
        if let Some(parent_name) = parent {
            let parent_idx = self.add_constant(alloc_string(parent_name));
            let parent_idx = u16::try_from(parent_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::LoadGlobal as u8, line);
            self.emit_bytes(&parent_idx.to_be_bytes(), line);
            self.emit_byte(OpCode::Inherit as u8, line);
        }

        // STORE_GLOBAL name → []（提前存储，使 class_vars / methods 可经 LOAD_GLOBAL 引用类自身）
        self.emit_byte(OpCode::StoreGlobal as u8, line);
        self.emit_bytes(&name_idx.to_be_bytes(), line);

        // 类属性：value; DUP; LOAD_GLOBAL name; SET_ATTR attr → [value]; POP
        // （SET_ATTR 约定 obj=pop(栈顶)，故 class 须在 value 之上，与 compile_store_target(Dot) 一致）
        for (attr_name, attr_expr) in class_vars {
            self.compile_expression(attr_expr, line)?;
            self.emit_byte(OpCode::Dup as u8, line);
            self.emit_byte(OpCode::LoadGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
            let attr_idx = self.add_constant(alloc_string(attr_name));
            let attr_idx = u16::try_from(attr_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::SetAttr as u8, line);
            self.emit_bytes(&attr_idx.to_be_bytes(), line);
            self.emit_byte(OpCode::Pop as u8, line);
        }

        // task 42：记录当前类名，供方法体内 Expr::SuperAccess 发射 GET_SUPER。
        let prev_class = self.current_class.take();
        self.current_class = Some(name.to_string());

        // 方法：LOAD_GLOBAL name; CLOSURE; METHOD name → [class]; POP
        for method in methods {
            let (m_name, m_params, m_body) =
                match method {
                    Stmt::FnDecl {
                        name, params, body, ..
                    } => (name, params, body),
                    _ => return Err(
                        "class body member must be a method or class variable (fn / name = expr)"
                            .into(),
                    ),
                };
            self.emit_byte(OpCode::LoadGlobal as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
            self.compile_function_closure(m_name, m_params, m_body, line, true, false)?;
            let m_idx = self.add_constant(alloc_string(m_name));
            let m_idx = u16::try_from(m_idx)
                .map_err(|_| "constant pool overflow: more than 65535 constants".to_string())?;
            self.emit_byte(OpCode::Method as u8, line);
            self.emit_bytes(&m_idx.to_be_bytes(), line);
            self.emit_byte(OpCode::Pop as u8, line);
        }

        self.current_class = prev_class;

        // 类已在上文 STORE_GLOBAL，无需重复存储。
        Ok(())
    }

    /// 编译 return：无值压入 NIL；单值直接求值；多值打包为元组。
    fn compile_return(&mut self, values: &[Expr], line: usize) -> Result<(), String> {
        match values.len() {
            0 => self.emit_byte(OpCode::Nil as u8, line),
            1 => self.compile_expression(&values[0], line)?,
            _ => {
                for v in values {
                    self.compile_expression(v, line)?;
                }
                let count = u8::try_from(values.len()).map_err(|_| {
                    format!("too many return values (max 255, got {})", values.len())
                })?;
                self.emit_byte(OpCode::BuildTuple as u8, line);
                self.emit_byte(count, line);
            }
        }
        // task 37：try body 内 return 须先注销所有外层 try 的 handler；
        // 补全（05-control-flow.md:85）：注销后按内→外内联各层 finally，
        // finally 语句净零栈效应，return 值保持栈顶；finally 体内抛出的
        // 异常向外传播（不被本层 except 捕获，与 Python 一致）。
        self.emit_early_exit_try_exits(line);
        let inlined: Vec<crate::ast::Stmt> = self
            .finally_stack
            .iter()
            .rev()
            .flat_map(|(_, fb)| fb.iter().cloned())
            .collect();
        for stmt in &inlined {
            self.compile_statement(stmt, line)?;
        }
        self.emit_return(line);
        Ok(())
    }

    /// 编译 defer 语句（task 36）。值绑定方案：求值 callee + 各参数（注册时求值，
    /// 满足规则 3），BUILD_TUPLE 打成单个 tuple，DEFER 弹出入当前帧 defer 栈。
    /// 要求 `expr` 为 Call 表达式（`defer f(args...)`）。
    fn compile_defer(&mut self, expr: &Expr, line: usize) -> Result<(), String> {
        let (callee, args) = match expr {
            Expr::Call { callee, args } => (callee.as_ref(), args.as_slice()),
            _ => return Err("SyntaxError: defer requires a call expression".into()),
        };
        self.compile_expression(callee, line)?;
        for arg in args {
            self.compile_expression(arg, line)?;
        }
        // tuple(callee, arg1, ..., argN)：count = N+1。
        let count = u8::try_from(args.len() + 1)
            .map_err(|_| format!("too many defer arguments (max 254, got {})", args.len()))?;
        self.emit_byte(OpCode::BuildTuple as u8, line);
        self.emit_byte(count, line);
        self.emit_byte(OpCode::Defer as u8, line);
        Ok(())
    }

    /// task 37：为当前点所有外层 try body 注销 handler（return/break/continue 出口）。
    /// emit 等量 TRY_EXIT。try_body 编译期间 try_depth > 0；except/finally 编译时
    /// try_depth 已恢复（其内 return/break/continue 不为本 try emit TRY_EXIT）。
    fn emit_early_exit_try_exits(&mut self, line: usize) {
        for _ in 0..self.try_depth {
            self.emit_byte(OpCode::TryExit as u8, line);
        }
    }

    /// task 37：编译 throw 语句。裸 throw → RETHROW；throw <expr> → 求值后 THROW。
    fn compile_throw(&mut self, expr: &Option<Expr>, line: usize) -> Result<(), String> {
        match expr {
            None => self.emit_byte(OpCode::Rethrow as u8, line),
            Some(e) => {
                self.compile_expression(e, line)?;
                self.emit_byte(OpCode::Throw as u8, line);
            }
        }
        Ok(())
    }

    /// task 37：编译 try/except/finally。字节码布局：
    ///
    /// ```text
    /// TRY_ENTER handler_off finally_off      ; 注册 handler（4 字节操作数）
    /// <try body>                             ; try_depth++ 期间编译（early-exit 插 TRY_EXIT）
    /// TRY_EXIT                               ; 正常完成，注销 handler
    /// JUMP finally_start / end
    /// dispatcher:                            ; catch_address：throw 跳到这里，栈顶 = 异常
    ///   <每个 typed except: CATCH/JUMP_IF_FALSE/POP；命中则 STORE/POP + body + CLEAR + JUMP>
    ///   <bare except: POP/STORE + body + CLEAR + JUMP>
    ///   no_match: POP, [JUMP finally_start | RETHROW]
    /// finally_start:                         ; （仅当有 finally）
    ///   <finally body>
    ///   FINALLY_END                          ; current_exc 非空则重抛
    /// end:
    /// ```
    fn compile_try(
        &mut self,
        try_block: &[Stmt],
        except_clauses: &[ExceptClause],
        finally_block: &Option<Vec<Stmt>>,
        line: usize,
    ) -> Result<(), String> {
        // TRY_ENTER：emit opcode + 4 字节占位（handler_off(2) + finally_off(2)）。
        self.emit_byte(OpCode::TryEnter as u8, line);
        let handler_patch = self.current_offset(); // handler_off 起始
        self.emit_bytes(&[0xff, 0xff], line);
        let finally_patch = self.current_offset(); // finally_off 起始
        self.emit_bytes(&[0xff, 0xff], line);
        let body_start = self.current_offset(); // TRY_ENTER 执行后 frame.ip 指向此处

        // try body（try_depth++ 使内部 return/break/continue 注销 handler）。
        // 含 finally 的 try 压入 finally_stack，供 early-exit 内联执行。
        self.try_depth += 1;
        if let Some(fb) = finally_block {
            self.finally_stack.push((self.try_depth, fb.to_vec()));
        }
        for stmt in try_block {
            self.compile_statement(stmt, line)?;
        }
        self.try_depth -= 1;

        // 正常完成：TRY_EXIT 注销 handler，跳过 dispatcher。
        self.emit_byte(OpCode::TryExit as u8, line);
        let normal_exit_jump = self.emit_jump(OpCode::Jump, line);

        // dispatcher：异常入口（throw 压异常到栈顶、设 current_exc、ip 跳此）。
        let dispatcher = self.current_offset();

        // 所有「跳到 finally/语句末尾」的跳转占位（正常完成 + 各命中分支 + no_match）。
        let mut to_target_jumps: Vec<usize> = Vec::new();

        for clause in except_clauses {
            // bare except（无类型）按 `except Error` 处理：所有内置异常皆派生自 Error，
            // 而 GeneratorExit 被 exception_matches 特判排除（05-control-flow.md:238），
            // 故 bare except 不会捕获 GeneratorExit。
            let type_str = match &clause.type_name {
                Some(path) => path.join("."),
                None => "Error".to_string(),
            };
            let name_idx = self.add_constant(alloc_string(&type_str));
            let name_idx = u16::try_from(name_idx)
                .map_err(|_| "too many constants for CATCH name".to_string())?;
            // CATCH（peek 异常、压 bool）→ JUMP_IF_FALSE no_match_landing。
            self.emit_byte(OpCode::Catch as u8, line);
            self.emit_bytes(&name_idx.to_be_bytes(), line);
            let jif = self.emit_jump(OpCode::JumpIfFalse, line);
            // 命中路径：POP bool。
            self.emit_byte(OpCode::Pop as u8, line);
            self.bind_or_pop_exc(clause, line)?;
            // 注意：CLEAR_CURRENT_EXC 置于 handler 体末尾（非开头）。handler 体内
            // 可能有裸 `throw`（RETHROW），需要 current_exc 仍持有当前异常。
            for stmt in &clause.body {
                self.compile_statement(stmt, line)?;
            }
            self.emit_byte(OpCode::ClearCurrentExc as u8, line);
            to_target_jumps.push(self.emit_jump(OpCode::Jump, line));
            // no_match 落点：JUMP_IF_FALSE 跳此（栈顶仍剩 bool）→ POP bool，落入下一 clause。
            self.patch_jump(jif)?;
            self.emit_byte(OpCode::Pop as u8, line);
        }

        // no_match：所有 except 均不匹配。栈顶为异常本体（current_exc 仍持有）。
        self.emit_byte(OpCode::Pop as u8, line); // 弹异常本体（current_exc 保留供重抛）
        if finally_block.is_some() {
            to_target_jumps.push(self.emit_jump(OpCode::Jump, line)); // → finally_start
        } else {
            self.emit_byte(OpCode::Rethrow as u8, line); // 无 finally → 重抛 current_exc
        }

        // finally_start：finally 体起始（无 finally 时与 end 重合）。
        let finally_start = self.current_offset();

        // 回填跳转：正常完成 + 各命中分支 + no_match(若有 finally) → finally_start。
        // 此时 code 末尾恰为 finally_start，故 patch_jump 目标正确；finally 体在其后发射。
        for jump in &to_target_jumps {
            self.patch_jump(*jump)?;
        }
        self.patch_jump(normal_exit_jump)?;

        // finally 体（若有）：执行后 FINALLY_END 决定续抛或 fall-through 到 end。
        // 编译 finally 体前弹出 finally_stack 条目——finally 内的 return 不再
        // 递归内联自身（只内联更外层）。
        if let Some(fb) = finally_block {
            self.finally_stack.pop();
            for stmt in fb {
                self.compile_statement(stmt, line)?;
            }
            self.emit_byte(OpCode::FinallyEnd as u8, line);
        }

        // 回填 TRY_ENTER 操作数：handler_off / finally_off 相对 body_start。
        let handler_off = u16::try_from(dispatcher.wrapping_sub(body_start))
            .map_err(|_| "try handler offset exceeds 65535".to_string())?;
        self.unit.chunk.code[handler_patch..handler_patch + 2]
            .copy_from_slice(&handler_off.to_be_bytes());
        match finally_block {
            Some(_) => {
                let fin_off = u16::try_from(finally_start.wrapping_sub(body_start))
                    .map_err(|_| "try finally offset exceeds 65535".to_string())?;
                self.unit.chunk.code[finally_patch..finally_patch + 2]
                    .copy_from_slice(&fin_off.to_be_bytes());
            }
            None => {
                // 0xFFFF 哨兵 = 无 finally。
                self.unit.chunk.code[finally_patch..finally_patch + 2]
                    .copy_from_slice(&[0xff, 0xff]);
            }
        }

        Ok(())
    }

    /// task 38：编译 with 语句（上下文管理器协议）。
    ///
    /// 字节码布局（见 docs/mslang/tasks/38-with-statement.md §2）。关键约定：
    /// - CALL 为 callee-below-args（`expression.rs:379-389`）：__enter__ 用 CALL 1，
    ///   __exit__ 用 CALL 4（self + err_type/err_msg/tb）。
    /// - handler 内不 emit TRY_EXIT：drive_unwind 命中 catch_address 时已 pop handler
    ///   （`src/vm/mod.rs` drive_unwind），再 emit 会空栈 pop。
    /// - try_depth++ 包裹 body，使内部 return/break/continue 插 TRY_EXIT 避免泄漏。
    /// - `as name` 在外围函数作用域注册（with 不创建新作用域，`03-syntax.md:595`）。
    /// - 用临时局部 `_with_ctx_N` 中转管理器，避免依赖不存在的 SWAP/ROT 指令。
    ///
    /// ```text
    /// <expr>                          ; 求值 → [ctx]
    /// STORE_LOCAL _with_ctx_N         ; → []
    /// LOAD_LOCAL _with_ctx_N          ; → [ctx]
    /// GET_ATTR "__enter__"            ; → [enter_fn]
    /// LOAD_LOCAL _with_ctx_N          ; → [enter_fn, ctx]
    /// CALL 1                          ; → [enter_result]
    /// STORE_LOCAL name | POP          ; as 绑定（外围作用域）或弹出 → []
    /// TRY_ENTER handler_off 0xFFFF    ; 无 finally
    /// <body ; try_depth++>
    /// TRY_EXIT                        ; 正常完成，注销 handler
    /// JUMP cleanup                    ; 跳过 cleanup_exc 的 POP
    /// cleanup_exc:                    ; 异常入口（drive_unwind 已设 current_exc、栈顶压异常）
    ///   POP                           ; 弹栈顶异常（current_exc 仍持有）
    /// cleanup:                        ; 正常/异常汇合
    ///   LOAD_LOCAL _with_ctx_N        ; → [ctx]
    ///   GET_ATTR "__exit__"           ; → [exit_fn]
    ///   LOAD_LOCAL _with_ctx_N        ; → [exit_fn, ctx]
    ///   LOAD_EXC_TYPE/MSG/TB          ; → [exit_fn, ctx, type|nil, msg|nil, tb|nil]
    ///   CALL 4                        ; → [exit_result]
    ///   LOAD_CURRENT_EXC              ; → [exit_result, exc_or_nil]
    ///   JUMP_IF_FALSE normal_done     ; nil（正常）→ 跳（不弹）
    ///   POP                           ; 异常路径：弹 exc
    ///   JUMP_IF_FALSE rethrow         ; exit_result 假 → 重抛（不弹）
    ///   POP; CLEAR_CURRENT_EXC; JUMP end   ; 抑制
    /// rethrow:   POP; LOAD_CURRENT_EXC; THROW
    /// normal_done: POP; POP           ; 弹 nil + exit_result
    /// end:
    /// ```
    fn compile_with(
        &mut self,
        expression: &Expr,
        alias: &Option<String>,
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        // —— 求值 expr，存入临时局部 _with_ctx_N（唯一名，支持嵌套）——
        self.compile_expression(expression, line)?;
        let tmp_name = format!("_with_ctx_{}", self.with_temp_counter);
        self.with_temp_counter += 1;
        self.declare_local(&tmp_name, line)?;
        let tmp_slot = u8::try_from(self.resolve_local(&tmp_name).ok_or_else(|| {
            format!(
                "internal: with temp local '{}' not found after declare",
                tmp_name
            )
        })?)
        .map_err(|_| "too many locals for with temp".to_string())?;
        self.emit_byte(OpCode::StoreLocal as u8, line);
        self.emit_byte(tmp_slot, line);

        // —— __enter__(ctx)：callee-below-args → CALL 1 ——
        self.emit_byte(OpCode::LoadLocal as u8, line);
        self.emit_byte(tmp_slot, line);
        self.emit_byte(OpCode::GetAttr as u8, line);
        let enter_idx = self.add_constant(alloc_string("__enter__"));
        let enter_idx =
            u16::try_from(enter_idx).map_err(|_| "too many constants for __enter__".to_string())?;
        self.emit_bytes(&enter_idx.to_be_bytes(), line);
        self.emit_byte(OpCode::LoadLocal as u8, line); // self 实参
        self.emit_byte(tmp_slot, line);
        self.emit_byte(OpCode::Call as u8, line);
        self.emit_byte(1, line);

        // —— `as name`：外围作用域注册（已存在则复用 slot）；否则 POP ——
        if let Some(name) = alias {
            let slot = match self.resolve_local(name) {
                Some(slot) => slot,
                None => {
                    self.declare_local(name, line)?;
                    self.resolve_local(name).ok_or_else(|| {
                        format!("internal: with alias '{}' not found after declare", name)
                    })?
                }
            };
            let slot =
                u8::try_from(slot).map_err(|_| "too many locals for with alias".to_string())?;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot, line);
        } else {
            self.emit_byte(OpCode::Pop as u8, line);
        }

        // —— TRY_ENTER：handler=cleanup_exc，无 finally（0xFFFF 哨兵）——
        self.emit_byte(OpCode::TryEnter as u8, line);
        let handler_patch = self.current_offset();
        self.emit_bytes(&[0xff, 0xff], line);
        let finally_patch = self.current_offset();
        self.emit_bytes(&[0xff, 0xff], line);
        let body_start = self.current_offset();

        // —— body（try_depth++：early-exit 插 TRY_EXIT，避免 handler 泄漏）——
        self.try_depth += 1;
        for stmt in body {
            self.compile_statement(stmt, line)?;
        }
        self.try_depth -= 1;

        // —— 正常完成：TRY_EXIT，跳到 cleanup（跳过 cleanup_exc 的 POP）——
        self.emit_byte(OpCode::TryExit as u8, line);
        let normal_jump = self.emit_jump(OpCode::Jump, line);

        // —— cleanup_exc：异常入口（栈顶=异常，current_exc 已设）——
        let cleanup_exc_addr = self.current_offset();
        self.emit_byte(OpCode::Pop as u8, line); // 弹栈顶异常（current_exc 仍持有）
                                                 // cleanup 合并点：normal_jump 跳到此处（跳过上面的 POP）。
        self.patch_jump(normal_jump)?;

        // —— __exit__(ctx, err_type, err_msg, tb)：callee-below-args → CALL 4 ——
        self.emit_byte(OpCode::LoadLocal as u8, line);
        self.emit_byte(tmp_slot, line);
        self.emit_byte(OpCode::GetAttr as u8, line);
        let exit_idx = self.add_constant(alloc_string("__exit__"));
        let exit_idx =
            u16::try_from(exit_idx).map_err(|_| "too many constants for __exit__".to_string())?;
        self.emit_bytes(&exit_idx.to_be_bytes(), line);
        self.emit_byte(OpCode::LoadLocal as u8, line); // self 实参
        self.emit_byte(tmp_slot, line);
        // 三异常参数从 current_exc 派生（无异常时压 nil）。
        self.emit_byte(OpCode::LoadExcType as u8, line);
        self.emit_byte(OpCode::LoadExcMsg as u8, line);
        self.emit_byte(OpCode::LoadExcTb as u8, line);
        self.emit_byte(OpCode::Call as u8, line);
        self.emit_byte(4, line);

        // —— 抑制/重抛判定（正常路径 current_exc=nil，整段无副作用地 POP 收尾）——
        self.emit_byte(OpCode::LoadCurrentExc as u8, line); // → [exit_result, exc_or_nil]
        let jif_normal = self.emit_jump(OpCode::JumpIfFalse, line);
        // 异常路径：POP exc，判定 exit_result 真值（JUMP_IF_FALSE 不弹栈）。
        self.emit_byte(OpCode::Pop as u8, line);
        let jif_rethrow = self.emit_jump(OpCode::JumpIfFalse, line);
        // 抑制（truthy）：POP exit_result，清 current_exc。
        self.emit_byte(OpCode::Pop as u8, line);
        self.emit_byte(OpCode::ClearCurrentExc as u8, line);
        let suppress_jump = self.emit_jump(OpCode::Jump, line);
        // 重抛（falsy）：POP exit_result，LOAD_CURRENT_EXC + THROW。
        self.patch_jump(jif_rethrow)?;
        self.emit_byte(OpCode::Pop as u8, line);
        self.emit_byte(OpCode::LoadCurrentExc as u8, line);
        self.emit_byte(OpCode::Throw as u8, line);
        // normal_done：[exit_result, nil]，POP nil + POP exit_result。
        self.patch_jump(jif_normal)?;
        self.emit_byte(OpCode::Pop as u8, line);
        self.emit_byte(OpCode::Pop as u8, line);
        // end。
        self.patch_jump(suppress_jump)?;

        // —— 回填 TRY_ENTER：handler_off 相对 body_start；finally=0xFFFF（已占位）——
        let handler_off = u16::try_from(cleanup_exc_addr.wrapping_sub(body_start))
            .map_err(|_| "with handler offset exceeds 65535".to_string())?;
        self.unit.chunk.code[handler_patch..handler_patch + 2]
            .copy_from_slice(&handler_off.to_be_bytes());
        let _ = finally_patch; // 0xFFFF 哨兵已就位（无 finally），无需回填
        Ok(())
    }

    /// task 37：绑定或弹出栈顶异常（except 命中分支）。有 alias 则 STORE_LOCAL，否则 POP。
    fn bind_or_pop_exc(&mut self, clause: &ExceptClause, line: usize) -> Result<(), String> {
        if let Some(alias) = &clause.alias {
            let slot = match self.resolve_local(alias) {
                Some(s) => s,
                None => {
                    self.declare_local(alias, line)?;
                    self.resolve_local(alias).ok_or_else(|| {
                        format!("internal: local '{}' not found after declare", alias)
                    })?
                }
            };
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot as u8, line);
        } else {
            self.emit_byte(OpCode::Pop as u8, line);
        }
        Ok(())
    }

    /// 编译 nonlocal 声明：仅在符号表中标记，不产生字节码。
    fn compile_nonlocal(&mut self, names: &[String], line: usize) -> Result<(), String> {
        for name in names {
            if self.global_names.contains(name) {
                return Err(format!(
                    "line {}: '{}' declared both nonlocal and global",
                    line, name
                ));
            }
            self.nonlocal_names.insert(name.clone());
        }
        Ok(())
    }

    /// 编译 global 声明：仅在符号表中标记，不产生字节码。
    fn compile_global(&mut self, names: &[String], line: usize) -> Result<(), String> {
        for name in names {
            if self.nonlocal_names.contains(name) {
                return Err(format!(
                    "line {}: '{}' declared both nonlocal and global",
                    line, name
                ));
            }
            self.global_names.insert(name.clone());
        }
        Ok(())
    }
}

// ---- 控制流：if / while / for..in / break / continue ----

impl Compiler {
    /// 编译 if/elif/else。
    fn compile_if(
        &mut self,
        condition: &Expr,
        then_block: &[Stmt],
        elif_clauses: &[(Expr, Vec<Stmt>)],
        else_block: &Option<Vec<Stmt>>,
        line: usize,
    ) -> Result<(), String> {
        let mut end_jumps = Vec::new();

        // 首个 if 分支
        self.compile_expression(condition, line)?;
        let else_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line);
        for stmt in then_block {
            self.compile_statement(stmt, line)?;
        }
        end_jumps.push(self.emit_jump(OpCode::Jump, line));
        self.patch_jump(else_jump)?;
        self.emit_byte(OpCode::Pop as u8, line);

        // elif 分支
        for (cond, body) in elif_clauses {
            self.compile_expression(cond, line)?;
            let next_jump = self.emit_jump(OpCode::JumpIfFalse, line);
            self.emit_byte(OpCode::Pop as u8, line);
            for stmt in body {
                self.compile_statement(stmt, line)?;
            }
            end_jumps.push(self.emit_jump(OpCode::Jump, line));
            self.patch_jump(next_jump)?;
            self.emit_byte(OpCode::Pop as u8, line);
        }

        // else 分支
        if let Some(else_body) = else_block {
            for stmt in else_body {
                self.compile_statement(stmt, line)?;
            }
        }

        // 所有分支汇合点
        for jump in end_jumps {
            self.patch_jump(jump)?;
        }
        Ok(())
    }

    /// 编译 while 循环。
    fn compile_while(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        let loop_start = self.current_offset();

        self.compile_expression(condition, line)?;
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse, line);
        self.emit_byte(OpCode::Pop as u8, line);

        self.current_loop.push(super::LoopContext {
            loop_start,
            break_jumps: Vec::new(),
            enclosing_try_depth: self.try_depth,
        });

        for stmt in body {
            self.compile_statement(stmt, line)?;
        }

        // 回边：跳回循环头重新检查条件
        let back_edge = self.emit_jump(OpCode::JumpBack, line);
        self.patch_jump_back(back_edge, loop_start)?;

        // 正常出口：条件为 false，跳到此处
        self.patch_jump(exit_jump)?;
        self.emit_byte(OpCode::Pop as u8, line);

        // 取出本循环的 break 跳转，patch 到出口（条件 POP 之后）
        let loop_ctx = self
            .current_loop
            .pop()
            .ok_or("internal: loop context stack underflow")?;
        for jump in &loop_ctx.break_jumps {
            self.patch_jump(*jump)?;
        }
        Ok(())
    }

    /// 编译 for..in 循环（单变量与双变量）。
    ///
    /// **迭代器存储在局部 slot 中**（非栈顶），通过 `FOR_ITER iter_slot offset`
    /// 从局部读取。这使嵌套 for..in 不冲突——每个循环的迭代器在各自的 slot 中，
    /// 不受内层循环栈操作影响（task 32 核心修复）。
    ///
    /// 栈布局（单变量 `for item in range(3)`，顶层）：
    /// ```text
    /// [Nil]      ← slot 0 (<self>)
    /// [Nil]      ← slot 1 (__for_iter_X，迭代器占位，后存入迭代器)
    /// [Nil]      ← slot 2 (item，循环变量占位)
    /// ```
    /// 编译 iterable 后 StoreLocal iter_slot 将迭代器写入 slot 1。
    /// FOR_ITER 从 slot 1 读取迭代器，压入 next 值至栈顶。
    /// StoreLocal 2 将值写入 item 的 slot，不影响迭代器。
    fn compile_for_in(
        &mut self,
        variable: &str,
        second_variable: Option<&str>,
        iterable: &Expr,
        body: &[Stmt],
        line: usize,
    ) -> Result<(), String> {
        // 1. 预留迭代器 slot（Nil 占位 + 声明隐藏局部；同名循环复用 slot）
        let iter_name = format!("__for_iter_{}", variable);
        let iter_slot = self.reserve_local_slot(&iter_name, line)?;

        let loop_start;
        let for_iter_exit;

        if let Some(var2) = second_variable {
            // 双变量：预留两个 slot
            let slot1 = self.reserve_local_slot(variable, line)?;
            let slot2 = self.reserve_local_slot(var2, line)?;

            // 2. 编译 iterable → ITERATOR → StoreLocal iter_slot
            self.compile_expression(iterable, line)?;
            self.emit_byte(OpCode::Iterator as u8, line);
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(iter_slot as u8, line);

            // 3. FOR_ITER 从 iter_slot 读取迭代器
            loop_start = self.current_offset();
            for_iter_exit = self.emit_for_iter(iter_slot as u8, line);
            self.emit_byte(OpCode::Unpack as u8, line);
            self.emit_byte(2, line);
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot1 as u8, line);
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot2 as u8, line);
        } else {
            // 单变量：预留一个 slot（同名循环复用）
            let slot = self.reserve_local_slot(variable, line)?;

            // 2. 编译 iterable → ITERATOR → StoreLocal iter_slot
            self.compile_expression(iterable, line)?;
            self.emit_byte(OpCode::Iterator as u8, line);
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(iter_slot as u8, line);

            // 3. FOR_ITER 从 iter_slot 读取迭代器
            loop_start = self.current_offset();
            for_iter_exit = self.emit_for_iter(iter_slot as u8, line);
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot as u8, line);
        }

        self.current_loop.push(super::LoopContext {
            loop_start,
            break_jumps: Vec::new(),
            enclosing_try_depth: self.try_depth,
        });

        for stmt in body {
            self.compile_statement(stmt, line)?;
        }

        let loop_ctx = self
            .current_loop
            .pop()
            .ok_or("internal: loop context stack underflow")?;

        // 回边：跳回 FOR_ITER
        let back_edge = self.emit_jump(OpCode::JumpBack, line);
        self.patch_jump_back(back_edge, loop_start)?;

        // 出口：FOR_ITER 耗尽与 break 跳到此处。迭代器在局部 slot 中，
        // 无需 Pop（与 task 26 栈顶方案不同）。
        self.patch_jump(for_iter_exit)?;
        for jump in &loop_ctx.break_jumps {
            self.patch_jump(*jump)?;
        }
        Ok(())
    }

    /// 编译 break：前向跳转到循环出口（由循环编译末尾统一 patch）。
    fn compile_break(&mut self, line: usize) -> Result<(), String> {
        let loop_base = self
            .current_loop
            .last()
            .ok_or_else(|| format!("line {}: 'break' outside loop", line))?
            .enclosing_try_depth;
        // 只注销循环体内注册的 handler（外层 try 的 handler 仍须保持活跃），
        // 并内联循环体内侧各层的 finally（depth > loop_base）。
        for _ in 0..(self.try_depth - loop_base) {
            self.emit_byte(OpCode::TryExit as u8, line);
        }
        let inlined: Vec<crate::ast::Stmt> = self
            .finally_stack
            .iter()
            .rev()
            .filter(|(depth, _)| *depth > loop_base)
            .flat_map(|(_, fb)| fb.iter().cloned())
            .collect();
        for stmt in &inlined {
            self.compile_statement(stmt, line)?;
        }
        let jump = self.emit_jump(OpCode::Break, line);
        self.current_loop
            .last_mut()
            .expect("loop context checked above")
            .break_jumps
            .push(jump);
        Ok(())
    }

    fn compile_continue(&mut self, line: usize) -> Result<(), String> {
        let (loop_start, loop_base) = {
            let ctx = self
                .current_loop
                .last()
                .ok_or_else(|| format!("line {}: 'continue' outside loop", line))?;
            (ctx.loop_start, ctx.enclosing_try_depth)
        };
        for _ in 0..(self.try_depth - loop_base) {
            self.emit_byte(OpCode::TryExit as u8, line);
        }
        let inlined: Vec<crate::ast::Stmt> = self
            .finally_stack
            .iter()
            .rev()
            .filter(|(depth, _)| *depth > loop_base)
            .flat_map(|(_, fb)| fb.iter().cloned())
            .collect();
        for stmt in &inlined {
            self.compile_statement(stmt, line)?;
        }
        let back = self.emit_jump(OpCode::Continue, line);
        self.patch_jump_back(back, loop_start)?;
        Ok(())
    }
}

// ---- task 59：select 语句编译 ----

impl Compiler {
    /// 编译 select 语句。按 case 源代码顺序预求值所有 channel 和 send value，
    /// 然后 SELECT 指令按 case 描述表进行多路复用。
    fn compile_select(
        &mut self,
        cases: &[SelectCase],
        default_block: &Option<Vec<Stmt>>,
        line: usize,
    ) -> Result<(), String> {
        if cases.len() > 255 {
            return Err("'select' case count exceeds 255".to_string());
        }

        // 1. 预求值：将 channel 和 send value 存入临时局部槽
        struct CaseDesc {
            kind: u8,
            channel_slot: u8,
            value_slot: u8,
            target_slot: u8,
        }

        let mut descs = Vec::with_capacity(cases.len());
        for (i, case) in cases.iter().enumerate() {
            match &case.operation {
                SelectOp::Receive { channel, target } => {
                    let ch_slot = self.resolve_or_emit_channel_slot(channel, line)?;
                    let tgt_slot = self.resolve_or_declare_target(target, line)?;
                    descs.push(CaseDesc {
                        kind: 0,
                        channel_slot: ch_slot,
                        value_slot: 0xFF,
                        target_slot: tgt_slot,
                    });
                }
                SelectOp::Send { channel, value } => {
                    let ch_slot = self.resolve_or_emit_channel_slot(channel, line)?;
                    // 同作用域多个 select 复用隐藏值 slot（同 for 循环的处理）；
                    // 占位 Nil 须在编译 value 前压栈，保证 StoreLocal 弹到 value 本体
                    let val_name = format!("__sel_val_{}", i);
                    let val_slot = self.reserve_local_slot(&val_name, line)? as u8;
                    self.compile_expression(value, line)?;
                    self.emit_byte(OpCode::StoreLocal as u8, line);
                    self.emit_byte(val_slot, line);
                    descs.push(CaseDesc {
                        kind: 1,
                        channel_slot: ch_slot,
                        value_slot: val_slot,
                        target_slot: 0xFF,
                    });
                }
            }
        }

        // 2. 发射 SELECT 指令
        let select_pc = self.unit.chunk.code.len();
        self.emit_byte(OpCode::Select as u8, line);
        self.emit_byte(cases.len() as u8, line);
        self.emit_byte(if default_block.is_some() { 1 } else { 0 }, line);

        // 3. 发射 case 描述表（每条 6 字节，body_offset 先占位）
        let table_start = self.unit.chunk.code.len();
        for desc in &descs {
            self.emit_byte(desc.kind, line);
            self.emit_byte(desc.channel_slot, line);
            self.emit_byte(desc.value_slot, line);
            self.emit_byte(desc.target_slot, line);
            self.emit_byte(0, line); // body_offset 高字节占位
            self.emit_byte(0, line); // body_offset 低字节占位
        }

        // 4. default_offset 占位
        let default_off_pos = self.unit.chunk.code.len();
        self.emit_byte(0, line);
        self.emit_byte(0, line);

        // 5. 依次编译 body，回填 body_offset
        let mut end_jumps = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            let body_pc = self.unit.chunk.code.len();
            let offset = body_pc as i64 - select_pc as i64;
            if !(-32768..=32767).contains(&offset) {
                return Err("'select' body offset exceeds 32KB".to_string());
            }
            let off_bytes = (offset as i16).to_be_bytes();
            let patch_pos = table_start + i * 6 + 4;
            self.unit.chunk.code[patch_pos] = off_bytes[0];
            self.unit.chunk.code[patch_pos + 1] = off_bytes[1];

            self.compile_block(&case.body, line)?;
            end_jumps.push(self.emit_jump(OpCode::Jump, line));
        }

        // 6. default body
        if let Some(db) = default_block {
            let d_pc = self.unit.chunk.code.len();
            let offset = d_pc as i64 - select_pc as i64;
            let off_bytes = (offset as i16).to_be_bytes();
            self.unit.chunk.code[default_off_pos] = off_bytes[0];
            self.unit.chunk.code[default_off_pos + 1] = off_bytes[1];
            self.compile_block(db, line)?;
            end_jumps.push(self.emit_jump(OpCode::Jump, line));
        }

        // 7. 回填所有 end jump → end_select
        for jmp in &end_jumps {
            self.patch_jump(*jmp)?;
        }

        Ok(())
    }

    /// 将 channel 名解析为局部槽索引。若已是局部变量则直接返回其槽；
    /// 若为全局/上值则发 LOAD 指令将其存入临时局部槽。
    fn resolve_or_emit_channel_slot(&mut self, name: &str, line: usize) -> Result<u8, String> {
        if let Some(slot) = self.resolve_local(name) {
            u8::try_from(slot).map_err(|_| "local slot exceeds 255".to_string())
        } else {
            self.emit_load_name(name, line)?;
            let temp = format!("__sel_ch_{}", self.unit.locals.len());
            self.declare_local(&temp, line)?;
            let slot = (self.unit.locals.len() - 1) as u8;
            self.emit_byte(OpCode::StoreLocal as u8, line);
            self.emit_byte(slot, line);
            Ok(slot)
        }
    }

    /// 解析 receive 目标变量槽。若已是局部则返回其槽；否则声明新局部。
    fn resolve_or_declare_target(&mut self, name: &str, line: usize) -> Result<u8, String> {
        if let Some(slot) = self.resolve_local(name) {
            u8::try_from(slot).map_err(|_| "local slot exceeds 255".to_string())
        } else {
            self.declare_local(name, line)?;
            Ok((self.unit.locals.len() - 1) as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::node::Program;
    use crate::compiler::{Chunk, Compiler, OpCode};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize_all().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    fn compile(source: &str) -> Chunk {
        let program = parse(source);
        let mut compiler = Compiler::new();
        compiler.compile(&program).unwrap()
    }

    #[test]
    fn test_compile_if_else() {
        let source = r#"
            x = 10
            if x > 5 {
                y = 1
            } else {
                y = 2
            }
        "#;
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(chunk.code.contains(&(OpCode::Jump as u8)));
        assert!(chunk.code.contains(&(OpCode::Halt as u8)));
    }

    #[test]
    fn test_compile_while() {
        let source = r#"
            i = 0
            while i < 3 {
                i += 1
            }
        "#;
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::JumpIfFalse as u8)));
        assert!(chunk.code.contains(&(OpCode::JumpBack as u8)));
    }

    #[test]
    fn test_compile_for_in_single_var() {
        let source = r#"
            for i in [1, 2, 3] {
                print(i)
            }
        "#;
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::Iterator as u8)));
        assert!(chunk.code.contains(&(OpCode::ForIter as u8)));
    }

    #[test]
    fn test_compile_for_in_two_vars() {
        let source = r#"
            for k, v in d.items() {
                print(k)
            }
        "#;
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::Unpack as u8)));
    }

    #[test]
    fn test_break_continue_use_dedicated_opcodes() {
        let source = r#"
            while true {
                break
            }
            for i in [1] {
                continue
            }
        "#;
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::Break as u8)));
        assert!(chunk.code.contains(&(OpCode::Continue as u8)));
    }

    #[test]
    fn test_break_outside_loop_is_error() {
        let program = parse("break");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_continue_outside_loop_is_error() {
        let program = parse("continue");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_all_jumps_patched() {
        let source = r#"
            x = 1
            if x > 0 {
                y = 1
            }
            for i in [1, 2] {
                if i == 1 {
                    continue
                }
                break
            }
        "#;
        let chunk = compile(source);
        let two_byte_jumps = [
            OpCode::Jump,
            OpCode::JumpIfFalse,
            OpCode::JumpIfTrue,
            OpCode::JumpBack,
            OpCode::Break,
            OpCode::Continue,
            OpCode::ForIter,
        ];
        for (i, &byte) in chunk.code.iter().enumerate() {
            if two_byte_jumps.iter().any(|op| *op as u8 == byte) {
                let offset = u16::from_be_bytes([chunk.code[i + 1], chunk.code[i + 2]]);
                assert_ne!(offset, 0xffff, "Unpatched jump {:?} at offset {}", byte, i);
            }
        }
    }

    #[test]
    fn test_var_decl_uses_store_local() {
        // var x = 10 → CONSTANT ... + STORE_LOCAL
        let chunk = compile("var x = 10");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::StoreLocal as u8)));
    }

    #[test]
    fn test_const_decl_valid_literal() {
        // const X = 10 通过常量表达式校验
        let chunk = compile("const X = 10");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::StoreLocal as u8)));
    }

    #[test]
    fn test_const_decl_valid_arithmetic() {
        // const 支持 + - * 等二元算术与括号
        let chunk = compile("const X = (1 + 2) * 3");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
    }

    #[test]
    fn test_const_decl_rejects_call() {
        // const 右侧出现函数调用 → 编译错误
        let program = parse("const X = foo()");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_const_decl_rejects_identifier() {
        // const 右侧出现变量引用 → 编译错误
        let program = parse("const X = y");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_return_with_no_value_emits_nil() {
        let chunk = compile("return");
        assert!(chunk.code.contains(&(OpCode::Nil as u8)));
        assert!(chunk.code.contains(&(OpCode::Return as u8)));
    }

    #[test]
    fn test_return_with_value() {
        let chunk = compile("return 42");
        assert!(chunk.code.contains(&(OpCode::Constant as u8)));
        assert!(chunk.code.contains(&(OpCode::Return as u8)));
    }

    // task 45：import/from-import 已实现，不再为 stub。
    // 原 test_deferred_statement_types_return_error 检查 import 编译报错，已移除。

    // ---- task 31：默认参数 / 可变参数编译期校验 ----

    #[test]
    fn test_param_order_default_before_positional_is_error() {
        // 普通参数不能出现在默认参数之后 → 编译错误。
        let program = parse("fn f(a = 1, b) { return a }");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_param_order_default_after_variadic_is_error() {
        // 默认参数不能出现在可变参数之后 → 编译错误。
        let program = parse("fn f(*rest, a = 1) { return a }");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_param_order_positional_after_variadic_is_error() {
        // 普通参数不能出现在可变参数之后 → 编译错误。
        let program = parse("fn f(*rest, a) { return a }");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_non_constant_default_is_error() {
        // 非常量默认值（如 []）暂不支持 → 编译错误。
        let program = parse("fn f(x = []) { return x }");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    #[test]
    fn test_constant_default_literals_compile_ok() {
        // 合法的默认值（int/float/string/bool/nil）与合法参数顺序 → 编译成功。
        let sources = [
            "fn f(a, b = 10) { return b }",
            "fn f(a, b = 3.14) { return b }",
            "fn f(a, b = \"hi\") { return b }",
            "fn f(a, b = true) { return b }",
            "fn f(a, b = nil) { return b }",
        ];
        for source in sources {
            let program = parse(source);
            let mut compiler = Compiler::new();
            assert!(
                compiler.compile(&program).is_ok(),
                "expected success for valid default: {:?}",
                source
            );
        }
    }

    // ---- task 36：defer 编译 ----

    #[test]
    fn test_compile_defer_emits_defer_opcode() {
        // defer f(args)：求值 callee+args → BUILD_TUPLE → DEFER。
        let source = "defer print(\"hi\")";
        let chunk = compile(source);
        assert!(chunk.code.contains(&(OpCode::Defer as u8)));
        assert!(chunk.code.contains(&(OpCode::BuildTuple as u8)));
    }

    #[test]
    fn test_compile_return_emits_exec_defer() {
        // 函数每个 RETURN 前须 emit EXEC_DEFER（含函数末尾隐式 RETURN）。
        // 函数体字节码在常量池中的 MsFunction 里（非顶层 chunk），需取出校验。
        use crate::vm::object::{read_function, Object, TypeTag};
        let source = "fn f() { return 1 }";
        let chunk = compile(source);
        let func_code = chunk
            .constants
            .iter()
            .find_map(|c| match c {
                Object::Ref(ptr) if unsafe { (**ptr).type_tag } == TypeTag::FUNCTION as u8 => {
                    Some(unsafe { read_function(*ptr) }.function.code.clone())
                }
                _ => None,
            })
            .expect("function constant not found");
        let ret_pos = func_code
            .iter()
            .position(|&b| b == OpCode::Return as u8)
            .expect("return opcode");
        assert!(ret_pos > 0);
        assert_eq!(func_code[ret_pos - 1], OpCode::ExecDefer as u8);
    }

    #[test]
    fn test_compile_top_level_emits_exec_defer_before_halt() {
        // 顶层返回点（HALT）前亦须 emit EXEC_DEFER（§8）。
        let source = "x = 1";
        let chunk = compile(source);
        let code = chunk.code;
        let halt_pos = code
            .iter()
            .position(|&b| b == OpCode::Halt as u8)
            .expect("halt opcode");
        assert!(halt_pos > 0);
        assert_eq!(code[halt_pos - 1], OpCode::ExecDefer as u8);
    }

    #[test]
    fn test_compile_defer_non_call_is_error() {
        let program = parse("defer 42");
        let mut compiler = Compiler::new();
        assert!(compiler.compile(&program).is_err());
    }

    // ---- task 44：装饰器编译 ----

    #[test]
    fn test_compile_decorator_emits_load_call_store() {
        // @dec\nfn f() {...} 应编译为：CLOSURE + STORE_GLOBAL(f) + LOAD_GLOBAL(dec)
        // + LOAD_GLOBAL(f) + CALL 1 + STORE_GLOBAL(f)。
        let source = "@dec\nfn f() {\n    return 1\n}\n";
        let chunk = compile(source);
        // 验证存在 CALL 1（装饰器应用）。
        let call_pos = chunk
            .code
            .windows(2)
            .position(|w| w[0] == OpCode::Call as u8 && w[1] == 1)
            .expect("CALL 1 for decorator application");
        // CALL 前应有 LOAD_GLOBAL（dec 和 f 各一次）。
        assert!(call_pos >= 2);
    }

    #[test]
    fn test_compile_multiple_decorators_two_calls() {
        // @d1\n@d2\nfn f() {} → 两次 CALL 1（d2 先应用，d1 后应用）。
        let source = "@d1\n@d2\nfn f() {\n    return 1\n}\n";
        let chunk = compile(source);
        let call_count = chunk
            .code
            .windows(2)
            .filter(|w| w[0] == OpCode::Call as u8 && w[1] == 1)
            .count();
        assert_eq!(call_count, 2, "expected 2 CALL 1 for 2 decorators");
    }

    #[test]
    fn test_compile_no_decorator_no_extra_call() {
        // 普通 fn 声明不应有装饰器的 CALL 1。
        let source = "fn f() {\n    return 1\n}\n";
        let chunk = compile(source);
        let call_count = chunk
            .code
            .windows(2)
            .filter(|w| w[0] == OpCode::Call as u8 && w[1] == 1)
            .count();
        assert_eq!(call_count, 0, "bare fn should have no decorator CALL");
    }
}
