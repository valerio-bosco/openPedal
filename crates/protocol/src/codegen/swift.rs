//! Swift output: `Codable` structs and enums matching the serde wire format exactly.

use std::fmt::Write;

use super::model::{Body, Def, Field, Kind, Tagged, Ty};
use super::{doc_lines, HEADER};

pub fn generate(defs: &[Def], consts: &[(&str, String)]) -> String {
    let mut o = String::new();
    writeln!(o, "{HEADER}\n\nimport Foundation\n").unwrap();
    writeln!(o, "enum OpenPedalProtocol {{").unwrap();
    for (name, value) in consts {
        let (n, v) = swift_const(name, value);
        writeln!(o, "    static let {n} = {v}").unwrap();
    }
    writeln!(o, "}}\n").unwrap();
    for d in defs {
        def(&mut o, d);
    }
    o.truncate(o.trim_end().len());
    o.push('\n');
    o
}

fn swift_const(name: &str, value: &str) -> (String, String) {
    let n = match name {
        "PROTOCOL_VERSION" => "version",
        "INPUT" => "input",
        "OUTPUT" => "output",
        "PEDAL_PREFIX" => "pedalPrefix",
        "PEDAL_IN" => "pedalIn",
        "PEDAL_OUT" => "pedalOut",
        other => panic!("no Swift name for constant {other}"),
    };
    (n.to_owned(), value.to_owned())
}

fn ty(t: &Ty) -> String {
    match t {
        Ty::Str => "String".into(),
        Ty::Num => "Double".into(),
        Ty::Int => "Int".into(),
        Ty::Bool => "Bool".into(),
        Ty::Ref(n) => n.clone(),
        Ty::Array(i) | Ty::Tuple(i, _) => format!("[{}]", ty(i)),
        Ty::Map(i) => format!("[String: {}]", ty(i)),
    }
}

fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Swift property name for a wire field (reserved words get a friendlier name).
fn prop(owner: &str, wire: &str) -> String {
    match (owner, wire) {
        ("StateMsg", "protocol") => "protocolVersion".into(),
        _ => camel(wire),
    }
}

fn swift_type_opt(f: &Field) -> String {
    let t = ty(&f.ty);
    if f.required && !f.nullable {
        t
    } else {
        format!("{t}?")
    }
}

fn docs(o: &mut String, indent: &str, d: &Option<String>) {
    for l in doc_lines(d) {
        writeln!(o, "{indent}/// {l}").unwrap();
    }
}

fn needs_keys(owner: &str, fs: &[Field]) -> bool {
    fs.iter().any(|f| prop(owner, &f.name) != f.name)
}

fn coding_keys(o: &mut String, indent: &str, enum_name: &str, owner: &str, fs: &[Field]) {
    let cases: Vec<String> = fs
        .iter()
        .map(|f| {
            let p = prop(owner, &f.name);
            let p = escape(&p);
            if p.trim_matches('`') == f.name {
                p
            } else {
                format!("{p} = \"{}\"", f.name)
            }
        })
        .collect();
    writeln!(o, "{indent}enum {enum_name}: String, CodingKey {{ case {} }}", cases.join(", ")).unwrap();
}

fn escape(p: &str) -> String {
    match p {
        "default" | "protocol" | "extension" | "operator" | "in" | "is" | "as" | "class" | "enum" | "func" | "let" | "var" | "init" => {
            format!("`{p}`")
        }
        _ => p.to_owned(),
    }
}

fn def(o: &mut String, d: &Def) {
    docs(o, "", &d.doc);
    let n = &d.name;
    match &d.kind {
        Kind::Struct(fs) => {
            writeln!(o, "struct {n}: Codable, Hashable {{").unwrap();
            for f in fs {
                docs(o, "    ", &f.doc);
                writeln!(o, "    var {}: {}", escape(&prop(n, &f.name)), swift_type_opt(f)).unwrap();
            }
            if needs_keys(n, fs) {
                writeln!(o).unwrap();
                coding_keys(o, "    ", "CodingKeys", n, fs);
            }
            writeln!(o, "}}\n").unwrap();
        }
        Kind::Flatten(fs, t) => {
            let kind_enum = format!("{n}{}", capitalize(&t.tag));
            writeln!(o, "struct {n}: Codable, Hashable {{").unwrap();
            for f in fs {
                docs(o, "    ", &f.doc);
                writeln!(o, "    var {}: {}", escape(&prop(n, &f.name)), swift_type_opt(f)).unwrap();
            }
            writeln!(o, "    var {}: {kind_enum}\n", escape(&camel(&t.tag))).unwrap();
            coding_keys(o, "    private ", "CodingKeys", n, fs);
            writeln!(o).unwrap();
            writeln!(o, "    init({}) {{", init_params(n, fs, &t.tag, &kind_enum)).unwrap();
            for f in fs {
                let p = escape(&prop(n, &f.name));
                writeln!(o, "        self.{p} = {p}").unwrap();
            }
            writeln!(o, "        self.{0} = {0}\n    }}\n", escape(&camel(&t.tag))).unwrap();
            writeln!(o, "    init(from decoder: Decoder) throws {{\n        let c = try decoder.container(keyedBy: CodingKeys.self)").unwrap();
            for f in fs {
                writeln!(o, "        {} = {}", escape(&prop(n, &f.name)), decode_expr(n, f, "c")).unwrap();
            }
            writeln!(o, "        {} = try {kind_enum}(from: decoder)\n    }}\n", escape(&camel(&t.tag))).unwrap();
            writeln!(o, "    func encode(to encoder: Encoder) throws {{\n        var c = encoder.container(keyedBy: CodingKeys.self)").unwrap();
            for f in fs {
                writeln!(o, "        {}", encode_expr(n, f, "c")).unwrap();
            }
            writeln!(o, "        try {}.encode(to: encoder)\n    }}\n}}\n", escape(&camel(&t.tag))).unwrap();
            tagged(o, &kind_enum, &t.tag, &t.variants_ref(), &format!("{n} {}", t.tag));
        }
        Kind::Tagged(t) => tagged(o, n, &t.tag, &t.variants_ref(), ""),
        Kind::StringEnum(vs) => {
            let cases: Vec<String> = vs.iter().map(|v| escape(&camel(v))).collect();
            writeln!(o, "enum {n}: String, Codable, Hashable, CaseIterable {{").unwrap();
            for (c, v) in cases.iter().zip(vs) {
                if c.trim_matches('`') == v {
                    writeln!(o, "    case {c}").unwrap();
                } else {
                    writeln!(o, "    case {c} = \"{v}\"").unwrap();
                }
            }
            writeln!(o, "}}\n").unwrap();
        }
        Kind::Untagged(ts) => {
            let cases: Vec<(String, String)> = ts.iter().map(|t| (case_name(t), ty(t))).collect();
            writeln!(o, "enum {n}: Codable, Hashable {{").unwrap();
            for (c, t) in &cases {
                writeln!(o, "    case {c}({t})").unwrap();
            }
            writeln!(o, "\n    init(from decoder: Decoder) throws {{\n        let c = try decoder.singleValueContainer()").unwrap();
            for (c, t) in &cases {
                writeln!(o, "        if let v = try? c.decode({t}.self) {{ self = .{c}(v); return }}").unwrap();
            }
            writeln!(o, "        throw DecodingError.dataCorruptedError(in: c, debugDescription: \"no variant of {n} matches\")\n    }}\n").unwrap();
            writeln!(o, "    func encode(to encoder: Encoder) throws {{\n        var c = encoder.singleValueContainer()\n        switch self {{").unwrap();
            for (c, _) in &cases {
                writeln!(o, "        case .{c}(let v): try c.encode(v)").unwrap();
            }
            writeln!(o, "        }}\n    }}\n}}\n").unwrap();
        }
        Kind::Alias(t) => writeln!(o, "typealias {n} = {}\n", ty(t)).unwrap(),
    }
}

fn case_name(t: &Ty) -> String {
    match t {
        Ty::Str => "string".into(),
        Ty::Num => "number".into(),
        Ty::Int => "integer".into(),
        Ty::Bool => "boolean".into(),
        Ty::Ref(n) => lower_first(n),
        _ => panic!("unsupported untagged member {t:?}"),
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn init_params(n: &str, fs: &[Field], tag: &str, kind_enum: &str) -> String {
    let mut ps: Vec<String> = fs
        .iter()
        .map(|f| {
            let def = if f.required && !f.nullable { "" } else { " = nil" };
            format!("{}: {}{def}", escape(&prop(n, &f.name)), swift_type_opt(f))
        })
        .collect();
    ps.push(format!("{}: {kind_enum}", escape(&camel(tag))));
    ps.join(", ")
}

fn decode_expr(owner: &str, f: &Field, c: &str) -> String {
    let key = format!(".{}", escape(&prop(owner, &f.name)));
    let t = ty(&f.ty);
    if f.required && !f.nullable {
        format!("try {c}.decode({t}.self, forKey: {key})")
    } else {
        format!("try {c}.decodeIfPresent({t}.self, forKey: {key})")
    }
}

fn encode_expr(owner: &str, f: &Field, c: &str) -> String {
    let p = escape(&prop(owner, &f.name));
    let key = format!(".{p}");
    if f.required && !f.nullable {
        format!("try {c}.encode({p}, forKey: {key})")
    } else {
        format!("try {c}.encodeIfPresent({p}, forKey: {key})")
    }
}

/// An internally tagged enum: `{"<tag>": "<variant>", ...fields}`.
fn tagged(o: &mut String, n: &str, tag: &str, variants: &[&super::model::Variant], _what: &str) {
    writeln!(o, "enum {n}: Codable, Hashable {{").unwrap();
    for v in variants {
        docs(o, "    ", &v.doc);
        let case = escape(&camel(&v.tag_value));
        match &v.body {
            Body::Unit => writeln!(o, "    case {case}").unwrap(),
            Body::Newtype(r) => writeln!(o, "    case {case}({r})").unwrap(),
            Body::Fields(fs) if fs.len() == 1 => writeln!(o, "    case {case}({})", swift_type_opt(&fs[0])).unwrap(),
            Body::Fields(fs) => {
                let ps: Vec<String> = fs.iter().map(|f| format!("{}: {}", escape(&camel(&f.name)), swift_type_opt(f))).collect();
                writeln!(o, "    case {case}({})", ps.join(", ")).unwrap();
            }
        }
    }
    writeln!(o, "\n    private enum TagKey: String, CodingKey {{ case {} }}", escape(tag)).unwrap();
    for v in variants {
        if let Body::Fields(fs) = &v.body {
            coding_keys(o, "    private ", &keys_name(v), "", fs);
        }
    }
    writeln!(o, "\n    init(from decoder: Decoder) throws {{").unwrap();
    writeln!(o, "        let tc = try decoder.container(keyedBy: TagKey.self)").unwrap();
    writeln!(o, "        switch try tc.decode(String.self, forKey: .{}) {{", escape(tag)).unwrap();
    for v in variants {
        let case = escape(&camel(&v.tag_value));
        writeln!(o, "        case \"{}\":", v.tag_value).unwrap();
        match &v.body {
            Body::Unit => writeln!(o, "            self = .{case}").unwrap(),
            Body::Newtype(r) => writeln!(o, "            self = .{case}(try {r}(from: decoder))").unwrap(),
            Body::Fields(fs) => {
                writeln!(o, "            let c = try decoder.container(keyedBy: {}.self)", keys_name(v)).unwrap();
                let args: Vec<String> = fs
                    .iter()
                    .map(|f| {
                        let e = decode_expr("", f, "c");
                        if fs.len() == 1 { e } else { format!("{}: {e}", camel(&f.name)) }
                    })
                    .collect();
                writeln!(o, "            self = .{case}({})", args.join(", ")).unwrap();
            }
        }
    }
    writeln!(
        o,
        "        case let other:\n            throw DecodingError.dataCorruptedError(forKey: .{}, in: tc, debugDescription: \"unknown {n} \\(other)\")\n        }}\n    }}\n",
        escape(tag)
    )
    .unwrap();
    writeln!(o, "    func encode(to encoder: Encoder) throws {{").unwrap();
    writeln!(o, "        var tc = encoder.container(keyedBy: TagKey.self)").unwrap();
    writeln!(o, "        switch self {{").unwrap();
    for v in variants {
        let case = escape(&camel(&v.tag_value));
        let tag_line = format!("try tc.encode(\"{}\", forKey: .{})", v.tag_value, escape(tag));
        match &v.body {
            Body::Unit => writeln!(o, "        case .{case}:\n            {tag_line}").unwrap(),
            Body::Newtype(_) => {
                writeln!(o, "        case .{case}(let v):\n            {tag_line}\n            try v.encode(to: encoder)").unwrap()
            }
            Body::Fields(fs) => {
                let binds: Vec<String> = fs.iter().map(|f| format!("let {}", escape(&camel(&f.name)))).collect();
                let pattern = if fs.len() == 1 { binds[0].clone() } else { binds.join(", ") };
                writeln!(o, "        case .{case}({pattern}):\n            {tag_line}").unwrap();
                writeln!(o, "            var c = encoder.container(keyedBy: {}.self)", keys_name(v)).unwrap();
                for f in fs {
                    let p = escape(&camel(&f.name));
                    let key = format!(".{}", escape(&camel(&f.name)));
                    let verb = if f.required && !f.nullable { "encode" } else { "encodeIfPresent" };
                    writeln!(o, "            try c.{verb}({p}, forKey: {key})").unwrap();
                }
            }
        }
    }
    writeln!(o, "        }}\n    }}\n}}\n").unwrap();
}

fn keys_name(v: &super::model::Variant) -> String {
    format!("{}Keys", capitalize(&camel(&v.tag_value)))
}

impl Tagged {
    fn variants_ref(&self) -> Vec<&super::model::Variant> {
        self.variants.iter().collect()
    }
}
