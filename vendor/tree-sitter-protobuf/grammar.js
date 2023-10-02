module.exports = grammar({
  name: 'protobuf',
  extras: ($) => [$.comment, /\s/],
  rules: {
    // top
    sourceFile: ($) =>
      seq($.syntax, repeat(choice($.import, $.extend, $.package, $.option, $.emptyStatement, $.enum, $.message, $.service))),
    
    // comment
    comment: $ => token(choice(
      seq('//', /.*/),
      seq(
        "/*",
        repeat(choice(
          /[^*]/,
          /\*[^/]/,
        )),
        "*/"
      )
    )),

    // syntax
    syntax: ($) => seq('syntax', '=', $.syntaxValue, ';'),
    syntaxValue: ($) => /"proto3"/,
    
    // extend
    extend: ($) => seq('extend', $.extendName, $.messageBody),

    // package
    package: ($) => seq('package', $.fullIdent, ';'),

    // imports
    import: ($) => seq('import', $.strLit, ';'),

    // option
    option: ($) => seq('option', $.optionName, '=', $.constant, ';'),
    optionName: ($) => choice(seq('(', $.fullIdent, ')'), $.fullIdent),

    // enum
    enum: ($) => seq('enum', $.enumName, $.enumBody),
    enumBody: ($) => seq('{', repeat(choice($.option, $.enumField, $.emptyStatement)), '}'),
    enumField: ($) =>
      seq(
        $.enumVariantName,
        '=',
        optional('-'),
        $.intLit,
        optional(seq('[', $.enumValueOption, repeat(seq(',', $.enumValueOption)), ']')),
        ';'
      ),
    enumValueOption: ($) => seq($.optionName, '=', $.constant),

    // message
    message: ($) => seq('message', $.messageName, $.messageBody),
    messageBody: ($) =>
      seq(
        '{',
        repeat(choice($.field, $.enum, $.message, $.option, $.oneof, $.mapField, $.reserved, $.emptyStatement)),
        '}'
      ),

    // service
    service: ($) => seq('service', $.serviceName, $.serviceBody),
    serviceBody: ($) => seq('{', repeat(choice($.option, $.rpc, $.emptyStatement)), '}'),
    rpc: ($) =>
      seq(
        'rpc',
        $.rpcName,
        '(',
        optional('stream'),
        $.enumMessageType,
        ')',
        'returns',
        '(',
        optional('stream'),
        $.enumMessageType,
        ')',
        $.rpcBody,
      ),
    rpcBody: ($) => choice(seq('{', repeat(choice($.option, $.emptyStatement)), '}'), ';'),

    // field and inline option
    field: ($) =>
      seq(optional('optional'), optional('repeated'), $.type, $.fieldName, '=', $.fieldNumber, optional(seq('[', $.fieldOptions, ']')), ';'),
    fieldOptions: ($) => seq($.fieldOption, repeat(seq(',', $.fieldOption))),
    fieldOption: ($) => seq($.optionName, '=', $.constant),

    // oneof
    oneof: ($) => seq('oneof', $.oneofName, $.oneofBody),
    oneofBody: ($) => seq('{', repeat(choice($.option, $.oneofField, $.emptyStatement)), '}'),
    oneofField: ($) => seq($.type, $.fieldName, '=', $.fieldNumber, optional(seq('[', $.fieldOptions, ']')), ';'),

    // map
    mapField: ($) =>
      seq(
        'map',
        '<',
        $.keyType,
        ',',
        $.type,
        '>',
        $.mapName,
        '=',
        $.fieldNumber,
        optional(seq('[', $.fieldOptions, ']')),
        ';'
      ),
    keyType: ($) =>
      choice(
        'int32',
        'int64',
        'uint32',
        'uint64',
        'sint32',
        'sint64',
        'fixed32',
        'fixed64',
        'sfixed32',
        'sfixed64',
        'bool',
        'string'
      ),

    // reserved
    reserved: ($) => seq('reserved', choice($.ranges, $.fieldNames)),
    ranges: ($) => seq($.range, repeat(seq(',', $.range))),
    range: ($) => seq($.intLit, optional(seq('to', choice($.intLit, 'max')))),
    fieldNames: ($) => seq($.fieldName, repeat(seq(',', $.fieldName))),

    // integer literals
    intLit: ($) => /(\d\d*|0[0-7]*|0[xX][\da-fA-F]*)/,

    // floating-point literals
    floatLit: ($) => choice(/\d\.\d*([eE][+-]\d*)?/, /\d*[eE][+-]\d*/, /\.\d*[eE][+-]\d*/, 'inf', 'nan'),

    // boolean literals
    boolLit: ($) => /(true|false)/,

    // string literals
    strLit: ($) =>
      choice(
        seq('"', /([^"\n\\]|\\[xX][\da-fA-F]{2}|\\[0-7]{3}|\\[abfnrtv\\'"])*/, '"'),
        seq("'", /([^'\n\\]|\\[xX][\da-fA-F]{2}|\\[0-7]{3}|\\[abfnrtv\\'"])*/, "'")
      ),

    // built-in field type
    type: ($) =>
      choice(
        'double',
        'float',
        'int32',
        'int64',
        'uint32',
        'uint64',
        'sint32',
        'sint64',
        'fixed32',
        'fixed64',
        'sfixed32',
        'sfixed64',
        'bool',
        'string',
        'bytes',
        $.enumMessageType
      ),
    fieldNumber: ($) => $.intLit,

    // empty statement
    emptyStatement: ($) => ';',

    // constant
    constant: ($) =>
      choice(
        $.fullIdent,
        seq(optional(/[+-]/), $.intLit),
        seq(optional(/[+-]/), $.floatLit),
        $.strLit,
        $.boolLit,
        $.msgLit
      ),
    msgLit: ($) => seq('{', repeat(seq($.fieldName, ':', $.constant)), '}'),

    // identifier
    ident: ($) => /[a-zA-Z_]\w*/,
    fullIdent: ($) => seq($.ident, repeat(seq('.', $.ident))),
    extendName: ($) => $.fullIdent,
    messageName: ($) => $.ident,
    mapName: ($) => $.ident,
    enumName: ($) => $.ident,
    fieldName: ($) => $.ident,
    enumVariantName: ($) => $.ident,
    oneofName: ($) => $.ident,
    serviceName: ($) => $.ident,
    rpcName: ($) => $.ident,
    enumMessageType: ($) => seq(optional('.'), repeat(seq($.ident, '.')), $.messageName),
  },
});
