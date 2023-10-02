[
  "syntax"
  "package"
  "option"
  "import"
  "service"
  "rpc"
  "returns"
  "message"
  "enum"
  "oneof"
  "repeated"
  "reserved"
  "to"
  "extend"
  "optional"
] @keyword

[
  (keyType)
  (type)
  (messageName)
  (enumName)
  (serviceName)
  (rpcName)
]@type

[
  (strLit)
  (syntaxValue)
] @string

[
  (intLit)
  (floatLit)
] @number

[
  (true)
  (false)
] @constant.builtin

(comment) @comment

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
]  @punctuation.bracket

