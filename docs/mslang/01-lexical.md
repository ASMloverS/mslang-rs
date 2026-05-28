# 词法规范

## 源文件

- 文件后缀：`.ms`
- 编码：UTF-8
- 换行符：`\n` 或 `\r\n` 均可（统一处理为 `\n`）

## 词法元素

### 关键字

```
var        const      fn         return
if         elif       else
while      for        in         break      continue
class      self       super
true       false      nil
and        or         not
try        except     finally    defer      with     throw
async      await      go
import     from       as
yield
nonlocal
```

共 **34** 个关键字。

### 标识符

```
identifier = [a-zA-Z_][a-zA-Z0-9_]*
```

- 首字符必须是字母或下划线
- 后续字符可以是字母、数字或下划线
- 区分大小写（`foo` 和 `Foo` 是不同的标识符）
- 关键字不可用作标识符

### 字面量

#### 整数字面量

```
integer = decimal | hex | binary | octal
decimal = [1-9][0-9]* | 0
hex     = 0[xX][0-9a-fA-F]+
binary  = 0[bB][01]+
octal   = 0[oO][0-7]+
```

示例：
```
42
0xFF
0b1010
0o755
```

#### 浮点字面量

```
float = [0-9]+ "." [0-9]+ ([eE][+-]?[0-9]+)?
      | [0-9]+ [eE][+-]?[0-9]+
```

示例：
```
3.14
2.0
1e10
1.5e-3
```

#### 布尔字面量

```
true
false
```

#### 空值字面量

```
nil
```

#### 字符串字面量

仅支持双引号字符串，不支持单引号。

```
string = '"' ( [^"\\] | escape )* '"'
escape = '\' ( '"' | '\' | 'n' | 't' | 'r' | '0' )
```

转义序列：

| 转义 | 含义 |
|---|---|
| `\"` | 双引号 `"` |
| `\\` | 反斜杠 `\` |
| `\n` | 换行 |
| `\t` | 制表符 |
| `\r` | 回车 |
| `\0` | 空字符 |

示例：
```
"hello world"
"line1\nline2"
"path: C:\\Users"
```

多行字符串暂不支持（后续版本考虑 `"""..."""`）。

### 运算符

#### 算术运算符

```
+  -  *  /  //  %  **
```

| 运算符 | 含义 |
|---|---|
| `+` | 加法 / 字符串拼接 / 正号 |
| `-` | 减法 / 负号 |
| `*` | 乘法 |
| `/` | 除法（浮点） |
| `//` | 整除 |
| `%` | 取模 |
| `**` | 幂运算 |

#### 比较运算符

```
==  !=  <  >  <=  >=
```

#### 逻辑运算符

```
and  or  not
```

`not` 为一元前缀运算符。

#### 位运算符

```
&  |  ^  <<  >>  ~
```

| 运算符 | 含义 |
|---|---|
| `&` | 按位与 |
| `|` | 按位或 |
| `^` | 按位异或 |
| `<<` | 左移 |
| `>>` | 右移 |
| `~` | 按位取反（一元） |

#### 成员运算符

```
in  is
```

| 运算符 | 含义 |
|---|---|
| `in` | 是否在集合中 |
| `is` | 是否为同一对象（身份比较） |

#### 赋值运算符

```
=  +=  -=  *=  /=  //=  %=  **=  &=  |=  ^=  <<=  >>=
```

#### 其他运算符

```
..  ...
```

| 运算符 | 含义 |
|---|---|
| `..` | 范围运算符（暂不使用，保留给未来版本） |
| `...` | 可变参数标记（暂不使用，保留给未来版本） |

### 分隔符

```
(  )  [  ]  {  }  ,  .  :  ;  ->
```

| 符号 | 含义 |
|---|---|
| `(` `)` | 分组 / 函数调用 |
| `[` `]` | 下标 / 列表字面量 |
| `{` `}` | 代码块 / dict/set 字面量 |
| `,` | 逗号分隔 |
| `.` | 属性访问 |
| `:` | dict键值对 / 切片分隔 |
| `;` | 保留，暂无语义 |
| `->` | 保留（函数返回类型标注） |

### 特殊语法符号

```
@  <-  :=
```

| 符号 | 含义 |
|---|---|
| `@` | 装饰器前缀 |
| `<-` | channel 接收（从 channel 读取值） |
| `:=` | 短变量声明 |

### 注释

仅支持 `#` 单行注释：

```
comment = "#" [^\n]*
```

注释从 `#` 开始到行尾结束，会被词法分析器跳过。

不支持多行注释 `/* ... */`。

### 空白符

```
whitespace = space | tab | newline
```

空白符用于分隔 token，但**不影响语义**（缩进无意义）。

换行符在某些上下文中作为语句终止符（见 [03-syntax](03-syntax.md)）。

## Token 完整列表

```rust
enum TokenKind {
    // 字面量
    Int(i64),
    Float(f64),
    String(String),

    // 标识符与关键字
    Identifier(String),
    // 34 个关键字
    Var, Const, Fn, Return,
    If, Elif, Else,
    While, For, In, Break, Continue,
    Class, Self, Super,
    True, False, Nil,
    And, Or, Not,
    Try, Except, Finally, Defer, With, Throw,
    Async, Await, Go,
    Import, From, As,
    Yield, Nonlocal,

    // 算术
    Plus, Minus, Star, Slash, DoubleSlash, Percent, DoubleStar,
    // 比较
    EqualEqual, BangEqual, Less, Greater, LessEqual, GreaterEqual,
    // 位运算
    Ampersand, Pipe, Caret, LeftShift, RightShift, Tilde,
    // 成员
    In, Is,
    // 赋值
    Equal, PlusEqual, MinusEqual, StarEqual, SlashEqual,
    DoubleSlashEqual, PercentEqual, DoubleStarEqual,
    AmpersandEqual, PipeEqual, CaretEqual, LeftShiftEqual, RightShiftEqual,
    // 短声明
    ColonEqual,
    // 分隔符
    LeftParen, RightParen,
    LeftBracket, RightBracket,
    LeftBrace, RightBrace,
    Comma, Dot, Colon, Semicolon, Arrow,
    // 特殊
    At, LeftArrow,

    // EOF
    Eof,
}
```

## 词法分析规则

### 最大匹配原则

词法分析器采用最长匹配：

- `**` 优先匹配为幂运算符
- `<<` 优先匹配为左移运算符
- `<=` 优先匹配为小于等于
- `//` 优先匹配为整除运算符
- `#` 开始到行尾为注释（不与整除冲突）

### 关键字 vs 标识符

词法分析器先匹配标识符，再查关键字表。若匹配到关键字，返回对应关键字 Token。

### 数值字面量优先级

1. `0x` / `0X` 开头 → 十六进制
2. `0b` / `0B` 开头 → 二进制
3. `0o` / `0O` 开头 → 八进制
4. 包含 `.` 或 `e`/`E` → 浮点数
5. 其他 → 十进制整数

### 字符串不可跨行

双引号字符串内不能包含未转义的换行符。跨行需使用 `\n` 转义。

## 保留字

以下标识符暂无语义，但保留给未来版本使用，不可用作变量名：

| 保留字 | 用途 |
|---|---|
| `select` | 多 channel 复用（见 [08-concurrency](08-concurrency.md)） |
| `default` | select 的默认分支 |
| `case` | select 的 case 分支 |
| `export` | 模块显式导出（见 [09-modules](09-modules.md)） |
| `match` | 模式匹配（预留） |
| `nonlocal` | 声明闭包内外层变量绑定（见 [03-syntax](03-syntax.md)） |
