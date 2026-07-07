import json

nested = json.parse("{\"a\": {\"b\": [1, 2, {\"c\": true}]}}")
print(nested["a"]["b"][2]["c"])

arr = json.parse("[1, null, \"hello\", [4, 5]]")
print(arr[1])
print(arr[3])

output = json.stringify({"list": [1, 2, 3], "nested": {"key": "val"}})
print(output)
