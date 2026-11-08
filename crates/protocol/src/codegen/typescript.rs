//! TypeScript output: interfaces/unions plus runtime type guards (used by the contract test).

use std::fmt::Write;

use super::model::{Body, Def, Field, Kind, Tagged, Ty};
use super::{doc_lines, HEADER};

pub fn generate(defs: &[Def], consts: &[(&str, String)]) -> String {
    let mut o = String::new();
    writeln!(o, "{HEADER}\n").unwrap();
    for (name, value) in consts {
        writeln!(o, "export const {name} = {value};").unwrap();
    }
    o.push('\n');
    for d in defs {
        types(&mut o, d);
    }
    o.push_str(GUARD_HELPERS);
    for d in defs {
        guard(&mut o, d);
    }
    o
}

const GUARD_HELPERS: &str = r#"// Runtime type guards. They check the shape of the wire format (required fields, types, tags);
// unknown extra fields are tolerated so the protocol can grow additively.

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

"#;

fn ts_ty(t: &Ty) -> String {
    match t {
        Ty::Str => "string".into(),
        Ty::Num | Ty::Int => "number".into(),
        Ty::Bool => "boolean".into(),
        Ty::Ref(n) => n.clone(),
        Ty::Array(i) => format!("{}[]", paren(&ts_ty(i))),
        Ty::Tuple(i, n) => format!("[{}]", vec![ts_ty(i); *n].join(", ")),
        Ty::Map(i) => format!("Record<string, {}>", ts_ty(i)),
    }
}

fn paren(s: &str) -> String {
    if s.contains(' ') && !s.starts_with("Record<") {
        format!("({s})")
    } else {
        s.to_owned()
    }
}

fn docs(o: &mut String, indent: &str, d: &Option<String>) {
    let lines = doc_lines(d);
    match lines.len() {
        0 => {}
        1 => writeln!(o, "{indent}/** {} */", lines[0]).unwrap(),
        _ => {
            writeln!(o, "{indent}/**").unwrap();
            for l in lines {
                writeln!(o, "{indent} * {l}").unwrap();
            }
            writeln!(o, "{indent} */").unwrap();
        }
    }
}

/// `name?: T;` for fields that may be absent. A nullable field that may be absent never
/// carries an explicit `null` on the wire (`skip_serializing_if`), so it is just optional.
fn field_ts(o: &mut String, indent: &str, f: &Field) {
    docs(o, indent, &f.doc);
    let opt = if f.required { "" } else { "?" };
    let null = if f.required && f.nullable { " | null" } else { "" };
    writeln!(o, "{indent}{}{opt}: {}{null};", f.name, ts_ty(&f.ty)).unwrap();
}

fn inline_fields(fs: &[Field], tag: Option<(&str, &str)>) -> String {
    let mut parts = Vec::new();
    if let Some((k, v)) = tag {
        parts.push(format!("{k}: \"{v}\""));
    }
    for f in fs {
        let opt = if f.required { "" } else { "?" };
        let null = if f.required && f.nullable { " | null" } else { "" };
        parts.push(format!("{}{opt}: {}{null}", f.name, ts_ty(&f.ty)));
    }
    format!("{{ {} }}", parts.join("; "))
}

fn tagged_ts(o: &mut String, t: &Tagged) {
    for v in &t.variants {
        docs(o, "  ", &v.doc);
        let body = match &v.body {
            Body::Unit => inline_fields(&[], Some((&t.tag, &v.tag_value))),
            Body::Fields(fs) => inline_fields(fs, Some((&t.tag, &v.tag_value))),
            Body::Newtype(r) => format!("({} & {r})", inline_fields(&[], Some((&t.tag, &v.tag_value)))),
        };
        writeln!(o, "  | {body}").unwrap();
    }
}

fn types(o: &mut String, d: &Def) {
    docs(o, "", &d.doc);
    match &d.kind {
        Kind::Struct(fs) => {
            writeln!(o, "export interface {} {{", d.name).unwrap();
            for f in fs {
                field_ts(o, "  ", f);
            }
            writeln!(o, "}}\n").unwrap();
        }
        Kind::Flatten(fs, t) => {
            writeln!(o, "export type {} = {} & (", d.name, inline_fields(fs, None)).unwrap();
            tagged_ts(o, t);
            writeln!(o, ");\n").unwrap();
        }
        Kind::Tagged(t) => {
            writeln!(o, "export type {} =", d.name).unwrap();
            tagged_ts(o, t);
            o.truncate(o.trim_end().len());
            writeln!(o, ";\n").unwrap();
        }
        Kind::StringEnum(vs) => {
            let u: Vec<String> = vs.iter().map(|v| format!("\"{v}\"")).collect();
            writeln!(o, "export type {} = {};\n", d.name, u.join(" | ")).unwrap();
        }
        Kind::Untagged(ts) => {
            let u: Vec<String> = ts.iter().map(ts_ty).collect();
            writeln!(o, "export type {} = {};\n", d.name, u.join(" | ")).unwrap();
        }
        Kind::Alias(t) => writeln!(o, "export type {} = {};\n", d.name, ts_ty(t)).unwrap(),
    }
}

// ---- guards -------------------------------------------------------------------------------

fn check(t: &Ty, x: &str, depth: usize) -> String {
    match t {
        Ty::Str => format!("typeof {x} === \"string\""),
        Ty::Num => format!("typeof {x} === \"number\""),
        Ty::Int => format!("Number.isInteger({x})"),
        Ty::Bool => format!("typeof {x} === \"boolean\""),
        Ty::Ref(n) => format!("is{n}({x})"),
        Ty::Array(i) => format!("Array.isArray({x}) && {x}.every((e{depth}) => {})", check(i, &format!("e{depth}"), depth + 1)),
        Ty::Tuple(i, n) => format!(
            "Array.isArray({x}) && {x}.length === {n} && {x}.every((e{depth}) => {})",
            check(i, &format!("e{depth}"), depth + 1)
        ),
        Ty::Map(i) => format!("isObj({x}) && Object.values({x}).every((e{depth}) => {})", check(i, &format!("e{depth}"), depth + 1)),
    }
}

fn field_checks(fs: &[Field], obj: &str) -> Vec<String> {
    fs.iter()
        .map(|f| {
            let x = format!("{obj}[\"{}\"]", f.name);
            let c = check(&f.ty, &x, 0);
            match (f.required, f.nullable) {
                (true, false) => c,
                (true, true) => format!("({x} === null || {c})"),
                (false, _) => format!("({x} === undefined || {c})"),
            }
        })
        .collect()
}

fn tagged_check(t: &Tagged) -> String {
    let arms: Vec<String> = t
        .variants
        .iter()
        .map(|v| {
            let mut parts = vec![format!("v[\"{}\"] === \"{}\"", t.tag, v.tag_value)];
            match &v.body {
                Body::Unit => {}
                Body::Fields(fs) => parts.extend(field_checks(fs, "v")),
                Body::Newtype(r) => parts.push(format!("is{r}(v)")),
            }
            format!("({})", parts.join(" && "))
        })
        .collect();
    format!("(\n    {}\n  )", arms.join(" ||\n    "))
}

fn guard(o: &mut String, d: &Def) {
    let n = &d.name;
    let body = match &d.kind {
        Kind::Struct(fs) => {
            let mut parts = vec!["isObj(v)".to_owned()];
            parts.extend(field_checks(fs, "v"));
            parts.join(" &&\n    ")
        }
        Kind::Flatten(fs, t) => {
            let mut parts = vec!["isObj(v)".to_owned()];
            parts.extend(field_checks(fs, "v"));
            parts.push(tagged_check(t));
            parts.join(" &&\n    ")
        }
        Kind::Tagged(t) => format!("isObj(v) &&\n    {}", tagged_check(t)),
        Kind::StringEnum(vs) => {
            let u: Vec<String> = vs.iter().map(|v| format!("\"{v}\"")).collect();
            format!("[{}].includes(v as string)", u.join(", "))
        }
        Kind::Untagged(ts) => ts.iter().map(|t| check(t, "v", 0)).collect::<Vec<_>>().join(" || "),
        Kind::Alias(t) => check(t, "v", 0),
    };
    writeln!(o, "export function is{n}(v: unknown): v is {n} {{\n  return {body};\n}}\n").unwrap();
}
