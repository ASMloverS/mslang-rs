import json

data = json.parse("{\"name\": \"Alice\", \"age\": 30}")
print(data["name"])
print(data["age"])

text = json.stringify({"x": 1, "y": [2, 3]})
print(text)
