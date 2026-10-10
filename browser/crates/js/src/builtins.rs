//! Level 1 standard objects. All callbacks return through the bounded VM.
use crate::value::*;
use crate::vm::{number_string, strict_equal, to_i32};
use crate::{ErrorKind, JsError, JsValue, Vm};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn error(s: &str) -> JsError {
    JsError::new(ErrorKind::TypeError, s)
}
fn arg(args: &[JsValue], i: usize) -> JsValue {
    args.get(i).cloned().unwrap_or(JsValue::Undefined)
}
fn num(vm: &mut Vm, args: &[JsValue], i: usize) -> Result<f64, JsError> {
    vm.to_number(arg(args, i))
}
fn text(vm: &mut Vm, args: &[JsValue], i: usize) -> Result<String, JsError> {
    vm.to_string(arg(args, i))
}
fn collect_strings<I, S>(vm: &mut Vm, strings: I) -> Result<Vec<JsValue>, JsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values = Vec::new();
    let mut bytes = 0usize;
    for string in strings {
        vm.tick()?;
        let string = string.as_ref();
        bytes = bytes.saturating_add(256).saturating_add(string.len());
        vm.check_temporary(bytes)?;
        values.push(JsValue::string(string));
    }
    Ok(values)
}
fn append_bounded(vm: &Vm, out: &mut String, part: &str) -> Result<(), JsError> {
    vm.check_string_bytes(out.len().saturating_add(part.len()))?;
    out.push_str(part);
    Ok(())
}
fn substitution(
    vm: &mut Vm,
    template: &str,
    input: &str,
    start: usize,
    end: usize,
    groups: &[JsValue],
) -> Result<String, JsError> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        vm.tick()?;
        if c == '$' {
            let part = match chars.peek().copied() {
                Some('$') => Some("$".to_owned()),
                Some('&') => Some(input[start..end].to_owned()),
                Some('`') => Some(input[..start].to_owned()),
                Some('\'') => Some(input[end..].to_owned()),
                Some(d @ '1'..='9') if (d as u32 - '0' as u32) < groups.len() as u32 => {
                    let mut index = (d as u32 - '0' as u32) as usize;
                    chars.next();
                    if let Some(d @ '0'..='9') = chars.peek().copied() {
                        let next = index * 10 + (d as u32 - '0' as u32) as usize;
                        if next < groups.len() {
                            chars.next();
                            index = next;
                        }
                    }
                    let value = &groups[index];
                    let text = if *value == JsValue::Undefined {
                        String::new()
                    } else {
                        vm.to_string(value.clone())?
                    };
                    append_bounded(vm, &mut out, &text)?;
                    continue;
                }
                _ => None,
            };
            if let Some(part) = part {
                chars.next();
                append_bounded(vm, &mut out, &part)?;
                continue;
            }
        }
        let mut encoded = [0; 4];
        append_bounded(vm, &mut out, c.encode_utf8(&mut encoded))?;
    }
    Ok(out)
}
fn method(vm: &mut Vm, id: ObjectId, key: &str, name: &str) {
    if let Ok(value) = vm.native(name) {
        let _ = vm.define(
            id,
            key.into(),
            Property {
                enumerable: false,
                ..Property::data(value)
            },
        );
    }
}
fn constant(vm: &mut Vm, id: ObjectId, key: &str, value: JsValue) {
    let _ = vm.define(
        id,
        key.into(),
        Property {
            writable: false,
            enumerable: false,
            configurable: false,
            ..Property::data(value)
        },
    );
}
pub(crate) fn install(vm: &mut Vm) {
    vm.array_proto = vm
        .allocate(ObjectKind::Array, Some(vm.object_proto))
        .unwrap_or(vm.object_proto);
    vm.string_proto = vm
        .allocate(
            ObjectKind::Boxed(JsValue::string("")),
            Some(vm.object_proto),
        )
        .unwrap_or(vm.object_proto);
    vm.number_proto = vm
        .allocate(
            ObjectKind::Boxed(JsValue::Number(0.0)),
            Some(vm.object_proto),
        )
        .unwrap_or(vm.object_proto);
    vm.boolean_proto = vm
        .allocate(
            ObjectKind::Boxed(JsValue::Bool(false)),
            Some(vm.object_proto),
        )
        .unwrap_or(vm.object_proto);
    vm.date_proto = vm
        .allocate(ObjectKind::Date(f64::NAN), Some(vm.object_proto))
        .unwrap_or(vm.object_proto);
    vm.regexp_proto = vm
        .allocate(ObjectKind::Ordinary, Some(vm.object_proto))
        .unwrap_or(vm.object_proto);
    vm.error_proto = vm
        .allocate(ObjectKind::Error, Some(vm.object_proto))
        .unwrap_or(vm.object_proto);
    for (name, proto) in [
        ("Object", vm.object_proto),
        ("Function", vm.function_proto),
        ("Array", vm.array_proto),
        ("String", vm.string_proto),
        ("Number", vm.number_proto),
        ("Boolean", vm.boolean_proto),
        ("Date", vm.date_proto),
        ("RegExp", vm.regexp_proto),
        ("Error", vm.error_proto),
    ] {
        if let Ok(constructor) = vm.native(name) {
            if let Some(id) = constructor.object() {
                let _ = vm.define(
                    id,
                    "prototype".into(),
                    Property {
                        enumerable: false,
                        ..Property::data(JsValue::Object(proto))
                    },
                );
                let _ = vm.define(
                    proto,
                    "constructor".into(),
                    Property {
                        enumerable: false,
                        ..Property::data(constructor.clone())
                    },
                );
            }
            let _ = vm.set_global(name, constructor);
        }
    }
    for name in [
        "TypeError",
        "ReferenceError",
        "SyntaxError",
        "RangeError",
        "EvalError",
        "URIError",
    ] {
        if let Ok(value) = vm.native(name) {
            if let Some(id) = value.object() {
                let _ = vm.define(
                    id,
                    "prototype".into(),
                    Property {
                        enumerable: false,
                        ..Property::data(JsValue::Object(vm.error_proto))
                    },
                );
            }
            let _ = vm.set_global(name, value);
        }
    }
    for key in [
        "toString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ] {
        method(vm, vm.object_proto, key, &format!("Object.prototype.{key}"));
    }
    for key in ["call", "apply", "bind", "toString"] {
        method(
            vm,
            vm.function_proto,
            key,
            &format!("Function.prototype.{key}"),
        );
    }
    for key in [
        "push",
        "pop",
        "shift",
        "unshift",
        "slice",
        "splice",
        "concat",
        "join",
        "toString",
        "indexOf",
        "lastIndexOf",
        "includes",
        "forEach",
        "map",
        "filter",
        "every",
        "some",
        "find",
        "findIndex",
        "reduce",
        "reduceRight",
        "reverse",
        "sort",
    ] {
        method(vm, vm.array_proto, key, &format!("Array.prototype.{key}"));
    }
    for key in [
        "toString",
        "valueOf",
        "charAt",
        "charCodeAt",
        "substring",
        "substr",
        "slice",
        "indexOf",
        "lastIndexOf",
        "includes",
        "startsWith",
        "endsWith",
        "trim",
        "toLowerCase",
        "toUpperCase",
        "split",
        "replace",
        "match",
        "search",
        "concat",
        "repeat",
    ] {
        method(vm, vm.string_proto, key, &format!("String.prototype.{key}"));
    }
    for key in [
        "toString",
        "valueOf",
        "toFixed",
        "toExponential",
        "toPrecision",
    ] {
        method(vm, vm.number_proto, key, &format!("Number.prototype.{key}"));
    }
    for key in ["toString", "valueOf"] {
        method(
            vm,
            vm.boolean_proto,
            key,
            &format!("Boolean.prototype.{key}"),
        );
    }
    for key in [
        "getTime",
        "valueOf",
        "toISOString",
        "toJSON",
        "toString",
        "getFullYear",
        "getUTCFullYear",
        "getMonth",
        "getUTCMonth",
        "getDate",
        "getUTCDate",
        "getDay",
        "getUTCDay",
        "getHours",
        "getUTCHours",
        "getMinutes",
        "getUTCMinutes",
        "getSeconds",
        "getUTCSeconds",
        "getMilliseconds",
        "getUTCMilliseconds",
        "getTimezoneOffset",
        "setTime",
    ] {
        method(vm, vm.date_proto, key, &format!("Date.prototype.{key}"));
    }
    for key in ["test", "exec", "toString"] {
        method(vm, vm.regexp_proto, key, &format!("RegExp.prototype.{key}"));
    }
    method(vm, vm.error_proto, "toString", "Error.prototype.toString");
    for (constructor, keys) in [
        (
            "Object",
            vec![
                "keys",
                "getOwnPropertyNames",
                "getOwnPropertySymbols",
                "create",
                "getPrototypeOf",
                "setPrototypeOf",
                "defineProperty",
                "defineProperties",
                "getOwnPropertyDescriptor",
                "assign",
                "freeze",
                "seal",
                "preventExtensions",
                "isExtensible",
                "isFrozen",
                "isSealed",
                "is",
            ],
        ),
        ("Array", vec!["isArray", "from", "of"]),
        ("String", vec!["fromCharCode", "fromCodePoint"]),
        (
            "Number",
            vec!["isNaN", "isFinite", "isInteger", "parseInt", "parseFloat"],
        ),
        ("Date", vec!["now", "parse", "UTC"]),
    ] {
        if let Ok(JsValue::Object(id)) = vm.get_global(constructor) {
            for key in keys {
                method(vm, id, key, &format!("{constructor}.{key}"));
            }
        }
    }
    if let Ok(JsValue::Object(id)) = vm.get_global("Number") {
        for (key, value) in [
            ("NaN", f64::NAN),
            ("POSITIVE_INFINITY", f64::INFINITY),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ("MAX_VALUE", f64::MAX),
            ("MIN_VALUE", f64::from_bits(1)),
            ("EPSILON", f64::EPSILON),
            ("MAX_SAFE_INTEGER", 9007199254740991.0),
        ] {
            constant(vm, id, key, JsValue::Number(value));
        }
    }
    let _ = vm.set_global("undefined", JsValue::Undefined);
    let _ = vm.set_global("NaN", JsValue::Number(f64::NAN));
    let _ = vm.set_global("Infinity", JsValue::Number(f64::INFINITY));
    for name in [
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "eval",
        "Symbol",
    ] {
        if let Ok(value) = vm.native(name) {
            let _ = vm.set_global(name, value);
        }
    }
    if let Ok(JsValue::Object(id)) = vm.get_global("Symbol") {
        let iterator = vm.symbol_next;
        vm.symbol_next += 1;
        constant(
            vm,
            id,
            "iterator",
            JsValue::Symbol(iterator, "Symbol.iterator".into()),
        );
        method(vm, id, "for", "Symbol.for");
        method(vm, id, "keyFor", "Symbol.keyFor");
    }
    for (name, keys) in [
        (
            "Math",
            vec![
                "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "exp", "floor", "log",
                "max", "min", "pow", "random", "round", "sin", "sqrt", "tan", "trunc", "sign",
                "log2", "log10",
            ],
        ),
        ("JSON", vec!["parse", "stringify"]),
        (
            "console",
            vec!["log", "info", "warn", "error", "debug", "assert"],
        ),
    ] {
        if let Ok(object) = vm.object() {
            if let Some(id) = object.object() {
                for key in keys {
                    method(vm, id, key, &format!("{name}.{key}"));
                }
                if name == "Math" {
                    for (key, n) in [
                        ("PI", std::f64::consts::PI),
                        ("E", std::f64::consts::E),
                        ("LN2", std::f64::consts::LN_2),
                        ("LN10", std::f64::consts::LN_10),
                        ("SQRT2", std::f64::consts::SQRT_2),
                    ] {
                        constant(vm, id, key, JsValue::Number(n));
                    }
                }
            }
            let _ = vm.set_global(name, object);
        }
    }
}

pub(crate) fn call(
    vm: &mut Vm,
    name: &str,
    this: JsValue,
    args: Vec<JsValue>,
    construct: bool,
    env: Option<usize>,
) -> Result<JsValue, JsError> {
    if name.starts_with("console.") {
        if name == "console.assert" && arg(&args, 0).truthy() {
            return Ok(JsValue::Undefined);
        }
        let mut parts = Vec::new();
        for value in &args {
            parts.push(
                vm.to_string(value.clone())
                    .unwrap_or_else(|_| "[Symbol]".into()),
            );
        }
        let line = parts.join(" ");
        if vm.console.len() < 1000 {
            vm.console.push(line.clone());
        }
        vm.emit_console(&line)?;
        return Ok(JsValue::Undefined);
    }
    if let Some(method) = name.strip_prefix("Array.prototype.") {
        return array_call(vm, method, this, &args);
    }
    if let Some(method) = name.strip_prefix("String.prototype.") {
        return string_call(vm, method, this, &args);
    }
    if let Some(method) = name.strip_prefix("Object.") {
        return object_call(vm, method, this, &args);
    }
    if let Some(method) = name.strip_prefix("Date.prototype.") {
        return date_call(vm, method, this, &args);
    }
    if let Some(method) = name.strip_prefix("RegExp.prototype.") {
        return regexp_call(vm, method, this, &args);
    }
    if let Some(method) = name.strip_prefix("Math.") {
        return math_call(vm, method, &args);
    }
    if let Some(method) = name.strip_prefix("Function.prototype.") {
        match method {
            "call" => {
                return vm.call_inner(
                    this,
                    arg(&args, 0),
                    args.iter().skip(1).cloned().collect(),
                    false,
                    None,
                )
            }
            "apply" => {
                let values = if matches!(arg(&args, 1), JsValue::Null | JsValue::Undefined) {
                    Vec::new()
                } else {
                    vm.array_values(arg(&args, 1))?
                };
                return vm.call_inner(this, arg(&args, 0), values, false, None);
            }
            "bind" => {
                if !vm.callable(&this) {
                    return Err(error("bind target not callable"));
                }
                return Ok(JsValue::Object(vm.allocate(
                    ObjectKind::Bound {
                        function: this,
                        this: arg(&args, 0),
                        args: args.iter().skip(1).cloned().collect(),
                    },
                    Some(vm.function_proto),
                )?));
            }
            "toString" => {
                return Ok(JsValue::string(
                    "function () { [native or interpreted code] }",
                ))
            }
            _ => return Err(error("unknown Function method")),
        }
    }
    if let Some(method) = name.strip_prefix("Number.prototype.") {
        let value = unbox(vm, this)?;
        let n = vm.to_number(value)?;
        return Ok(match method {
            "valueOf" => JsValue::Number(n),
            "toFixed" => {
                let digits = if args.is_empty() {
                    0
                } else {
                    num(vm, &args, 0)? as usize
                };
                if digits > 100 {
                    return Err(JsError::new(
                        ErrorKind::RangeError,
                        "precision out of range",
                    ));
                }
                JsValue::string(format!("{n:.digits$}"))
            }
            "toExponential" => {
                let digits = if args.is_empty() {
                    6
                } else {
                    num(vm, &args, 0)? as usize
                };
                if digits > 100 {
                    return Err(JsError::new(
                        ErrorKind::RangeError,
                        "precision out of range",
                    ));
                }
                JsValue::string(format!("{n:.digits$e}"))
            }
            "toPrecision" | "toString" => {
                if method == "toString" && !args.is_empty() && num(vm, &args, 0)? != 10.0 {
                    let radix = num(vm, &args, 0)? as u32;
                    if !(2..=36).contains(&radix) {
                        return Err(JsError::new(ErrorKind::RangeError, "radix out of range"));
                    }
                    let mut v = n.abs() as u64;
                    let mut out = String::new();
                    loop {
                        let d = (v % radix as u64) as u32;
                        if let Some(c) = char::from_digit(d, radix) {
                            out.insert(0, c);
                        }
                        v /= radix as u64;
                        if v == 0 {
                            break;
                        }
                    }
                    if n < 0.0 {
                        out.insert(0, '-');
                    }
                    JsValue::string(out)
                } else {
                    JsValue::string(number_string(n))
                }
            }
            _ => return Err(error("unknown Number method")),
        });
    }
    if let Some(method) = name.strip_prefix("Boolean.prototype.") {
        let value = unbox(vm, this)?;
        let b = value.truthy();
        return Ok(if method == "valueOf" {
            JsValue::Bool(b)
        } else {
            JsValue::string(b.to_string())
        });
    }
    match name {
        "Object" => {
            let value = arg(&args, 0);
            if let JsValue::Object(_) = value {
                return Ok(value);
            }
            if matches!(value, JsValue::Null | JsValue::Undefined) {
                vm.object()
            } else {
                let prototype = match value {
                    JsValue::String(_) => vm.string_proto,
                    JsValue::Bool(_) => vm.boolean_proto,
                    _ => vm.number_proto,
                };
                Ok(JsValue::Object(
                    vm.allocate(ObjectKind::Boxed(value), Some(prototype))?,
                ))
            }
        }
        "Array" => {
            if args.len() == 1 {
                if let JsValue::Number(n) = args[0] {
                    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 || n > 1_000_000.0 {
                        return Err(JsError::new(ErrorKind::RangeError, "array length limit"));
                    }
                    let array = vm.array(Vec::new())?;
                    vm.set(array.clone(), "length", JsValue::Number(n))?;
                    return Ok(array);
                }
            }
            vm.array(args)
        }
        "Array.isArray" => Ok(JsValue::Bool(
            arg(&args, 0)
                .object()
                .and_then(|id| vm.heap.get(id).ok())
                .is_some_and(|o| matches!(o.kind, ObjectKind::Array)),
        )),
        "Array.of" => vm.array(args),
        "Array.from" => {
            let values = vm.array_values(arg(&args, 0))?;
            if vm.callable(&arg(&args, 1)) {
                let mut mapped = Vec::new();
                for (i, v) in values.into_iter().enumerate() {
                    mapped.push(vm.call_inner(
                        arg(&args, 1),
                        arg(&args, 2),
                        vec![v, JsValue::Number(i as f64)],
                        false,
                        None,
                    )?);
                }
                vm.array(mapped)
            } else {
                vm.array(values)
            }
        }
        "String" => {
            let s = if args.is_empty() {
                String::new()
            } else if let JsValue::Symbol(_, description) = &args[0] {
                format!("Symbol({description})")
            } else {
                vm.to_string(args[0].clone())?
            };
            let value = JsValue::string(s);
            if construct {
                Ok(JsValue::Object(vm.allocate(
                    ObjectKind::Boxed(value),
                    Some(vm.string_proto),
                )?))
            } else {
                Ok(value)
            }
        }
        "String.fromCharCode" => {
            let mut units = Vec::new();
            for value in args {
                units.push(to_i32(vm.to_number(value)?) as u16);
            }
            Ok(JsValue::string(String::from_utf16_lossy(&units)))
        }
        "String.fromCodePoint" => {
            let mut out = String::new();
            for value in args {
                let n = vm.to_number(value)?;
                let ch = char::from_u32(n as u32)
                    .filter(|_| n >= 0.0 && n.fract() == 0.0)
                    .ok_or_else(|| JsError::new(ErrorKind::RangeError, "invalid code point"))?;
                out.push(ch);
            }
            Ok(JsValue::string(out))
        }
        "Number" => {
            let value = JsValue::Number(if args.is_empty() {
                0.0
            } else {
                vm.to_number(args[0].clone())?
            });
            if construct {
                Ok(JsValue::Object(vm.allocate(
                    ObjectKind::Boxed(value),
                    Some(vm.number_proto),
                )?))
            } else {
                Ok(value)
            }
        }
        "Boolean" => {
            let value = JsValue::Bool(arg(&args, 0).truthy());
            if construct {
                Ok(JsValue::Object(vm.allocate(
                    ObjectKind::Boxed(value),
                    Some(vm.boolean_proto),
                )?))
            } else {
                Ok(value)
            }
        }
        "isNaN" => Ok(JsValue::Bool(num(vm, &args, 0)?.is_nan())),
        "isFinite" => Ok(JsValue::Bool(num(vm, &args, 0)?.is_finite())),
        "Number.isNaN" => Ok(JsValue::Bool(
            matches!(arg(&args,0),JsValue::Number(n)if n.is_nan()),
        )),
        "Number.isFinite" => Ok(JsValue::Bool(
            matches!(arg(&args,0),JsValue::Number(n)if n.is_finite()),
        )),
        "Number.isInteger" => Ok(JsValue::Bool(
            matches!(arg(&args,0),JsValue::Number(n)if n.is_finite()&&n.fract()==0.0),
        )),
        "parseInt" | "Number.parseInt" => {
            let source = text(vm, &args, 0)?;
            let mut source = source.trim_start();
            let negative = source.starts_with('-');
            if source.starts_with(['+', '-']) {
                source = &source[1..];
            }
            let mut radix = if args.len() > 1 {
                to_i32(num(vm, &args, 1)?)
            } else {
                0
            };
            if radix == 0 {
                radix = if source.starts_with("0x") || source.starts_with("0X") {
                    16
                } else {
                    10
                };
            }
            if !(2..=36).contains(&radix) {
                return Ok(JsValue::Number(f64::NAN));
            }
            if radix == 16 && (source.starts_with("0x") || source.starts_with("0X")) {
                source = &source[2..];
            }
            let mut n = 0f64;
            let mut any = false;
            for ch in source.chars() {
                let Some(d) = ch.to_digit(radix as u32) else {
                    break;
                };
                n = n * radix as f64 + d as f64;
                any = true;
            }
            Ok(JsValue::Number(if any {
                if negative {
                    -n
                } else {
                    n
                }
            } else {
                f64::NAN
            }))
        }
        "parseFloat" | "Number.parseFloat" => {
            let source = text(vm, &args, 0)?;
            let source = source.trim_start();
            if source.starts_with("Infinity") || source.starts_with("+Infinity") {
                return Ok(JsValue::Number(f64::INFINITY));
            }
            if source.starts_with("-Infinity") {
                return Ok(JsValue::Number(f64::NEG_INFINITY));
            }
            let bytes = source.as_bytes();
            let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
            let mut digits = 0;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
                digits += 1;
                vm.tick()?;
            }
            if bytes.get(end) == Some(&b'.') {
                end += 1;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                    digits += 1;
                    vm.tick()?;
                }
            }
            if digits == 0 {
                return Ok(JsValue::Number(f64::NAN));
            }
            if matches!(bytes.get(end), Some(b'e' | b'E')) {
                let exponent = end;
                end += 1;
                if matches!(bytes.get(end), Some(b'+' | b'-')) {
                    end += 1;
                }
                let start = end;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                    vm.tick()?;
                }
                if end == start {
                    end = exponent;
                }
            }
            Ok(JsValue::Number(source[..end].parse().unwrap_or(f64::NAN)))
        }
        "Symbol" => {
            if construct {
                return Err(error("Symbol is not a constructor"));
            }
            let description = if args.is_empty() {
                String::new()
            } else {
                text(vm, &args, 0)?
            };
            let id = vm.symbol_next;
            vm.symbol_next += 1;
            Ok(JsValue::Symbol(id, description.into()))
        }
        "Symbol.for" => {
            let key = text(vm, &args, 0)?;
            let global = vm.get_global("Symbol")?;
            let private = format!("\0registry:{key}");
            let old = vm.get(global.clone(), &private)?;
            if old != JsValue::Undefined {
                return Ok(old);
            }
            let id = vm.symbol_next;
            vm.symbol_next += 1;
            let value = JsValue::Symbol(id, key.into());
            vm.set(global, &private, value.clone())?;
            Ok(value)
        }
        "Symbol.keyFor" => {
            let value = arg(&args, 0);
            if !matches!(value, JsValue::Symbol(..)) {
                return Err(error("keyFor expects Symbol"));
            }
            let global = vm.get_global("Symbol")?;
            for key in vm.keys(&global, false)? {
                if let PropertyKey::String(k) = key {
                    if k.starts_with("\0registry:") && vm.get(global.clone(), &k)? == value {
                        return Ok(JsValue::string(&k[10..]));
                    }
                }
            }
            Ok(JsValue::Undefined)
        }
        "RegExp" => {
            if let Some(id) = arg(&args, 0).object() {
                if let ObjectKind::RegExp { source, flags, .. } = vm.heap.get(id)?.kind.clone() {
                    let flags = if args.len() > 1 {
                        text(vm, &args, 1)?
                    } else {
                        flags
                    };
                    return vm.regexp(&source, &flags);
                }
            }
            let source = if args.is_empty() {
                String::new()
            } else {
                text(vm, &args, 0)?
            };
            let flags = if args.len() > 1 {
                text(vm, &args, 1)?
            } else {
                String::new()
            };
            vm.regexp(&source, &flags)
        }
        "Date" => {
            let n = if args.is_empty() {
                now()
            } else if args.len() == 1 {
                match arg(&args, 0) {
                    JsValue::String(s) => parse_date(&s),
                    value => vm.to_number(value)?,
                }
            } else {
                date_components(vm, &args)?
            };
            if construct {
                Ok(JsValue::Object(vm.allocate(
                    ObjectKind::Date(clip_date(n)),
                    Some(vm.date_proto),
                )?))
            } else {
                Ok(JsValue::string(date_string(now())))
            }
        }
        "Date.now" => Ok(JsValue::Number(now())),
        "Date.parse" => Ok(JsValue::Number(parse_date(&text(vm, &args, 0)?))),
        "Date.UTC" => Ok(JsValue::Number(clip_date(date_components(vm, &args)?))),
        "Error" | "TypeError" | "ReferenceError" | "SyntaxError" | "RangeError" | "EvalError"
        | "URIError" => {
            let id = vm.allocate(ObjectKind::Error, Some(vm.error_proto))?;
            let message = if args.is_empty() {
                String::new()
            } else {
                text(vm, &args, 0)?
            };
            vm.define(id, "name".into(), Property::data(JsValue::string(name)))?;
            vm.define(
                id,
                "message".into(),
                Property::data(JsValue::string(message)),
            )?;
            Ok(JsValue::Object(id))
        }
        "Error.prototype.toString" => {
            let name = vm.get(this.clone(), "name")?;
            let message = vm.get(this, "message")?;
            let name = vm.to_string(name)?;
            let message = vm.to_string(message)?;
            let result = if message.is_empty() {
                name
            } else {
                format!("{name}: {message}")
            };
            Ok(JsValue::string(result))
        }
        "JSON.parse" => {
            let source = text(vm, &args, 0)?;
            let json: serde_json::Value = serde_json::from_str(&source)
                .map_err(|e| JsError::new(ErrorKind::SyntaxError, e.to_string()))?;
            from_json(vm, json, 0)
        }
        "JSON.stringify" => {
            let value = to_json(vm, arg(&args, 0), &mut HashSet::new(), 0, &mut 0)?;
            if let Some(value) = value {
                Ok(JsValue::string(
                    if args.len() > 2 {
                        serde_json::to_string_pretty(&value)
                    } else {
                        serde_json::to_string(&value)
                    }
                    .map_err(|e| error(&e.to_string()))?,
                ))
            } else {
                Ok(JsValue::Undefined)
            }
        }
        "eval" => {
            if let JsValue::String(source) = arg(&args, 0) {
                vm.dynamic_eval(&source, env)
            } else {
                Ok(arg(&args, 0))
            }
        }
        "Function" => {
            let mut parts = Vec::new();
            for value in &args {
                parts.push(vm.to_string(value.clone())?);
            }
            let body = parts.pop().unwrap_or_default();
            let source = format!("(function anonymous({}){{{body}}});", parts.join(","));
            vm.dynamic_eval(&source, None)
        }
        _ => Err(error(&format!("unknown native function {name}"))),
    }
}
fn unbox(vm: &Vm, value: JsValue) -> Result<JsValue, JsError> {
    if let Some(id) = value.object() {
        if let ObjectKind::Boxed(value) = &vm.heap.get(id)?.kind {
            return Ok(value.clone());
        }
    }
    Ok(value)
}
fn object_call(
    vm: &mut Vm,
    method: &str,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    match method {
        "prototype.valueOf" => Ok(this),
        "prototype.toString" => {
            let tag = match &this {
                JsValue::Undefined => "Undefined",
                JsValue::Null => "Null",
                JsValue::String(_) => "String",
                JsValue::Number(_) => "Number",
                JsValue::Bool(_) => "Boolean",
                JsValue::Symbol(..) => "Symbol",
                JsValue::Object(id) => match &vm.heap.get(*id)?.kind {
                    ObjectKind::Array => "Array",
                    ObjectKind::Date(_) => "Date",
                    ObjectKind::RegExp { .. } => "RegExp",
                    ObjectKind::Function(_) | ObjectKind::Native(_) | ObjectKind::Bound { .. } => {
                        "Function"
                    }
                    ObjectKind::Error => "Error",
                    ObjectKind::Host { class, .. } => class,
                    _ => "Object",
                },
            };
            Ok(JsValue::string(format!("[object {tag}]")))
        }
        "prototype.hasOwnProperty" | "prototype.propertyIsEnumerable" => {
            let key = vm.property_key(arg(args, 0))?;
            let result = this
                .object()
                .and_then(|id| vm.heap.get(id).ok())
                .and_then(|o| o.properties.get(&key))
                .is_some_and(|p| method == "prototype.hasOwnProperty" || p.enumerable);
            Ok(JsValue::Bool(result))
        }
        "prototype.isPrototypeOf" => {
            let target = this.object();
            let mut current = arg(args, 0)
                .object()
                .and_then(|id| vm.heap.get(id).ok())
                .and_then(|o| o.prototype);
            let mut seen = HashSet::new();
            while let Some(id) = current {
                if Some(id) == target {
                    return Ok(JsValue::Bool(true));
                }
                if !seen.insert(id) {
                    break;
                }
                current = vm.heap.get(id)?.prototype;
            }
            Ok(JsValue::Bool(false))
        }
        "keys" | "getOwnPropertyNames" | "getOwnPropertySymbols" => {
            let keys = vm.keys(&arg(args, 0), method == "keys")?;
            let values = keys
                .into_iter()
                .filter_map(|k| match k {
                    PropertyKey::String(s) if method != "getOwnPropertySymbols" => {
                        Some(JsValue::string(s))
                    }
                    PropertyKey::Symbol(id) if method == "getOwnPropertySymbols" => {
                        Some(JsValue::Symbol(id, "".into()))
                    }
                    _ => None,
                })
                .collect();
            vm.array(values)
        }
        "create" => {
            let proto = arg(args, 0);
            if !matches!(proto, JsValue::Object(_) | JsValue::Null) {
                return Err(error("prototype must be object or null"));
            }
            let object = JsValue::Object(vm.allocate(ObjectKind::Ordinary, proto.object())?);
            if args.len() > 1 {
                define_properties(vm, object.clone(), arg(args, 1))?;
            }
            Ok(object)
        }
        "getPrototypeOf" => {
            let id = arg(args, 0)
                .object()
                .ok_or_else(|| error("getPrototypeOf expects object"))?;
            Ok(vm
                .heap
                .get(id)?
                .prototype
                .map(JsValue::Object)
                .unwrap_or(JsValue::Null))
        }
        "setPrototypeOf" => {
            let object = arg(args, 0);
            let id = object
                .object()
                .ok_or_else(|| error("setPrototypeOf target"))?;
            let proto = arg(args, 1);
            if !matches!(proto, JsValue::Object(_) | JsValue::Null) {
                return Err(error("invalid prototype"));
            }
            let mut current = proto.object();
            let mut seen = HashSet::new();
            while let Some(p) = current {
                if p == id || !seen.insert(p) {
                    return Err(error("cyclic prototype"));
                }
                current = vm.heap.get(p)?.prototype;
            }
            vm.heap.get_mut(id)?.prototype = proto.object();
            Ok(object)
        }
        "defineProperty" => {
            let object = arg(args, 0);
            let id = object
                .object()
                .ok_or_else(|| error("defineProperty expects object"))?;
            let key = vm.property_key(arg(args, 1))?;
            let descriptor = descriptor(vm, arg(args, 2))?;
            vm.define(id, key, descriptor)?;
            Ok(object)
        }
        "defineProperties" => {
            let object = arg(args, 0);
            define_properties(vm, object.clone(), arg(args, 1))?;
            Ok(object)
        }
        "getOwnPropertyDescriptor" => {
            let object = arg(args, 0);
            let id = object.object().ok_or_else(|| error("descriptor target"))?;
            let key = vm.property_key(arg(args, 1))?;
            let Some(property) = vm.heap.get(id)?.properties.get(&key).cloned() else {
                return Ok(JsValue::Undefined);
            };
            let result = vm.object()?;
            match property.value {
                PropertyValue::Data(v) => {
                    vm.set(result.clone(), "value", v)?;
                    vm.set(result.clone(), "writable", JsValue::Bool(property.writable))?;
                }
                PropertyValue::Accessor { get, set } => {
                    vm.set(result.clone(), "get", get)?;
                    vm.set(result.clone(), "set", set)?;
                }
            }
            vm.set(
                result.clone(),
                "enumerable",
                JsValue::Bool(property.enumerable),
            )?;
            vm.set(
                result.clone(),
                "configurable",
                JsValue::Bool(property.configurable),
            )?;
            Ok(result)
        }
        "assign" => {
            let object = arg(args, 0);
            if object.object().is_none() {
                return Err(error("assign target"));
            }
            for source in args.iter().skip(1) {
                for key in vm.keys(source, true)? {
                    let value = vm.get_key(source.clone(), key.clone())?;
                    vm.set_key(object.clone(), key, value)?;
                }
            }
            Ok(object)
        }
        "freeze" | "seal" | "preventExtensions" => {
            let value = arg(args, 0);
            if let Some(id) = value.object() {
                let object = vm.heap.get_mut(id)?;
                object.extensible = false;
                for p in object.properties.values_mut() {
                    if method != "preventExtensions" {
                        p.configurable = false;
                    }
                    if method == "freeze" {
                        p.writable = false;
                    }
                }
            }
            Ok(value)
        }
        "isExtensible" | "isFrozen" | "isSealed" => {
            let result = if let Some(id) = arg(args, 0).object() {
                let o = vm.heap.get(id)?;
                match method {
                    "isExtensible" => o.extensible,
                    "isFrozen" => {
                        !o.extensible
                            && o.properties
                                .values()
                                .all(|p| !p.configurable && !p.writable)
                    }
                    _ => !o.extensible && o.properties.values().all(|p| !p.configurable),
                }
            } else {
                method != "isExtensible"
            };
            Ok(JsValue::Bool(result))
        }
        "is" => {
            let a = arg(args, 0);
            let b = arg(args, 1);
            Ok(JsValue::Bool(match (&a, &b) {
                (JsValue::Number(a), JsValue::Number(b)) if a.is_nan() && b.is_nan() => true,
                (JsValue::Number(a), JsValue::Number(b)) if *a == 0.0 && *b == 0.0 => {
                    a.is_sign_negative() == b.is_sign_negative()
                }
                _ => strict_equal(&a, &b),
            }))
        }
        _ => Err(error("unknown Object method")),
    }
}
fn descriptor(vm: &mut Vm, value: JsValue) -> Result<Property, JsError> {
    if value.object().is_none() {
        return Err(error("property descriptor must be object"));
    }
    let get = vm.get(value.clone(), "get")?;
    let set = vm.get(value.clone(), "set")?;
    let data =
        vm.has(value.clone(), "value".into())? || vm.has(value.clone(), "writable".into())?;
    let accessor = get != JsValue::Undefined || set != JsValue::Undefined;
    if data && accessor {
        return Err(error("mixed property descriptor"));
    }
    if accessor
        && ((get != JsValue::Undefined && !vm.callable(&get))
            || (set != JsValue::Undefined && !vm.callable(&set)))
    {
        return Err(error("invalid accessor"));
    }
    Ok(Property {
        value: if accessor {
            PropertyValue::Accessor { get, set }
        } else {
            PropertyValue::Data(vm.get(value.clone(), "value")?)
        },
        writable: vm.get(value.clone(), "writable")?.truthy(),
        enumerable: vm.get(value.clone(), "enumerable")?.truthy(),
        configurable: vm.get(value, "configurable")?.truthy(),
    })
}
fn define_properties(vm: &mut Vm, target: JsValue, source: JsValue) -> Result<(), JsError> {
    let id = target.object().ok_or_else(|| error("descriptor target"))?;
    let mut properties = Vec::new();
    for key in vm.keys(&source, true)? {
        let value = vm.get_key(source.clone(), key.clone())?;
        properties.push((key, descriptor(vm, value)?));
    }
    for (key, p) in properties {
        vm.define(id, key, p)?;
    }
    Ok(())
}
fn array_call(
    vm: &mut Vm,
    method: &str,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let len = vm.array_length(this.clone())?;
    match method {
        "push" => {
            for (i, value) in args.iter().enumerate() {
                vm.set(this.clone(), &(len + i).to_string(), value.clone())?;
            }
            vm.set(this, "length", JsValue::Number((len + args.len()) as f64))?;
            Ok(JsValue::Number((len + args.len()) as f64))
        }
        "pop" => {
            if len == 0 {
                return Ok(JsValue::Undefined);
            }
            let value = vm.get(this.clone(), &(len - 1).to_string())?;
            vm.delete(this.clone(), (len - 1).to_string().into())?;
            vm.set(this, "length", JsValue::Number((len - 1) as f64))?;
            Ok(value)
        }
        "shift" | "unshift" => {
            let mut values = vm.array_values(this.clone())?;
            let result = if method == "shift" {
                if values.is_empty() {
                    JsValue::Undefined
                } else {
                    values.remove(0)
                }
            } else {
                let mut next = args.to_vec();
                next.extend(values);
                values = next;
                JsValue::Number(values.len() as f64)
            };
            replace_array(vm, this, &values)?;
            Ok(result)
        }
        "join" | "toString" => {
            let separator = if method == "toString" || args.is_empty() {
                ",".into()
            } else {
                text(vm, args, 0)?
            };
            let mut out = String::new();
            for (i, value) in vm.array_values(this)?.into_iter().enumerate() {
                let part = if matches!(value, JsValue::Null | JsValue::Undefined) {
                    String::new()
                } else {
                    vm.to_string(value)?
                };
                let extra = part
                    .len()
                    .saturating_add(if i == 0 { 0 } else { separator.len() });
                vm.check_string_bytes(out.len().saturating_add(extra))?;
                if i != 0 {
                    out.push_str(&separator);
                }
                out.push_str(&part);
            }
            Ok(JsValue::string(out))
        }
        "slice" => {
            let start = index(num_default(vm, args, 0, 0.0)?, len);
            let end = index(num_default(vm, args, 1, len as f64)?, len);
            let mut values = Vec::new();
            for i in start..end.max(start) {
                values.push(vm.get(this.clone(), &i.to_string())?);
            }
            vm.array(values)
        }
        "splice" => {
            let start = index(num_default(vm, args, 0, 0.0)?, len);
            let delete = if args.len() < 2 {
                len - start
            } else {
                num(vm, args, 1)?.max(0.0) as usize
            }
            .min(len - start);
            let mut values = vm.array_values(this.clone())?;
            let removed = values
                .splice(start..start + delete, args.iter().skip(2).cloned())
                .collect::<Vec<_>>();
            replace_array(vm, this, &values)?;
            vm.array(removed)
        }
        "concat" => {
            let mut out = vm.array_values(this)?;
            for value in args {
                if value
                    .object()
                    .and_then(|id| vm.heap.get(id).ok())
                    .is_some_and(|o| matches!(o.kind, ObjectKind::Array))
                {
                    out.extend(vm.array_values(value.clone())?);
                } else {
                    out.push(value.clone());
                }
            }
            vm.array(out)
        }
        "indexOf" | "lastIndexOf" | "includes" => {
            let target = arg(args, 0);
            let from = num_default(
                vm,
                args,
                1,
                if method == "lastIndexOf" {
                    len.saturating_sub(1) as f64
                } else {
                    0.0
                },
            )?;
            let start = if method == "lastIndexOf" {
                if from < 0.0 {
                    (len as f64 + from).max(-1.0) as isize
                } else {
                    (from as usize).min(len.saturating_sub(1)) as isize
                }
            } else {
                index(from, len) as isize
            };
            let mut indices = if method == "lastIndexOf" {
                (0..=start).rev().collect::<Vec<_>>()
            } else {
                (start..len as isize).collect()
            };
            for i in indices.drain(..) {
                let value = vm.get(this.clone(), &i.to_string())?;
                if strict_equal(&value, &target)
                    || method == "includes"
                        && matches!((&value,&target),(JsValue::Number(a),JsValue::Number(b))if a.is_nan()&&b.is_nan())
                {
                    return Ok(if method == "includes" {
                        JsValue::Bool(true)
                    } else {
                        JsValue::Number(i as f64)
                    });
                }
            }
            Ok(if method == "includes" {
                JsValue::Bool(false)
            } else {
                JsValue::Number(-1.0)
            })
        }
        "forEach" | "map" | "filter" | "every" | "some" | "find" | "findIndex" => {
            let callback = arg(args, 0);
            if !vm.callable(&callback) {
                return Err(error("array callback is not callable"));
            }
            let mut out = Vec::new();
            for i in 0..len {
                let key: PropertyKey = i.to_string().into();
                if !vm.has(this.clone(), key)? && !matches!(method, "find" | "findIndex") {
                    if method == "map" {
                        out.push(JsValue::Undefined);
                    }
                    continue;
                }
                let value = vm.get(this.clone(), &i.to_string())?;
                let result = vm.call_inner(
                    callback.clone(),
                    arg(args, 1),
                    vec![value.clone(), JsValue::Number(i as f64), this.clone()],
                    false,
                    None,
                )?;
                match method {
                    "map" => out.push(result),
                    "filter" if result.truthy() => out.push(value),
                    "every" if !result.truthy() => return Ok(JsValue::Bool(false)),
                    "some" if result.truthy() => return Ok(JsValue::Bool(true)),
                    "find" if result.truthy() => return Ok(value),
                    "findIndex" if result.truthy() => return Ok(JsValue::Number(i as f64)),
                    _ => {}
                }
            }
            match method {
                "map" | "filter" => vm.array(out),
                "every" => Ok(JsValue::Bool(true)),
                "some" => Ok(JsValue::Bool(false)),
                "findIndex" => Ok(JsValue::Number(-1.0)),
                _ => Ok(JsValue::Undefined),
            }
        }
        "reduce" | "reduceRight" => {
            let callback = arg(args, 0);
            if !vm.callable(&callback) {
                return Err(error("reduce callback"));
            }
            let mut indices = (0..len).collect::<Vec<_>>();
            if method == "reduceRight" {
                indices.reverse();
            }
            let mut acc = args.get(1).cloned();
            for i in indices {
                if !vm.has(this.clone(), i.to_string().into())? {
                    continue;
                }
                let value = vm.get(this.clone(), &i.to_string())?;
                acc = Some(if let Some(acc) = acc {
                    vm.call_inner(
                        callback.clone(),
                        JsValue::Undefined,
                        vec![acc, value, JsValue::Number(i as f64), this.clone()],
                        false,
                        None,
                    )?
                } else {
                    value
                });
            }
            acc.ok_or_else(|| error("reduce of empty array"))
        }
        "reverse" => {
            let mut values = vm.array_values(this.clone())?;
            values.reverse();
            replace_array(vm, this.clone(), &values)?;
            Ok(this)
        }
        "sort" => {
            let mut values = vm.array_values(this.clone())?;
            let callback = arg(args, 0);
            for i in 1..values.len() {
                let mut j = i;
                while j > 0 {
                    vm.tick()?;
                    let less = if vm.callable(&callback) {
                        let n = vm.call_inner(
                            callback.clone(),
                            JsValue::Undefined,
                            vec![values[j].clone(), values[j - 1].clone()],
                            false,
                            None,
                        )?;
                        vm.to_number(n)? < 0.0
                    } else {
                        vm.to_string(values[j].clone())? < vm.to_string(values[j - 1].clone())?
                    };
                    if !less {
                        break;
                    }
                    values.swap(j, j - 1);
                    j -= 1;
                }
            }
            replace_array(vm, this.clone(), &values)?;
            Ok(this)
        }
        _ => Err(error("unknown Array method")),
    }
}
fn replace_array(vm: &mut Vm, this: JsValue, values: &[JsValue]) -> Result<(), JsError> {
    vm.set(this.clone(), "length", JsValue::Number(0.0))?;
    for (i, value) in values.iter().enumerate() {
        vm.set(this.clone(), &i.to_string(), value.clone())?;
    }
    vm.set(this, "length", JsValue::Number(values.len() as f64))
}
fn index(n: f64, len: usize) -> usize {
    if n.is_nan() {
        0
    } else if n < 0.0 {
        (len as f64 + n).max(0.0) as usize
    } else {
        (n as usize).min(len)
    }
}
fn num_default(vm: &mut Vm, args: &[JsValue], i: usize, default: f64) -> Result<f64, JsError> {
    if matches!(arg(args, i), JsValue::Undefined) {
        Ok(default)
    } else {
        num(vm, args, i)
    }
}
fn string_call(
    vm: &mut Vm,
    method: &str,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let value = unbox(vm, this)?;
    if matches!(value, JsValue::Null | JsValue::Undefined) {
        return Err(error("String method receiver"));
    }
    let s = vm.to_string(value)?;
    let units = s.encode_utf16().collect::<Vec<_>>();
    let len = units.len();
    match method {
        "valueOf" | "toString" => Ok(JsValue::string(s)),
        "charAt" | "charCodeAt" => {
            let n = num_default(vm, args, 0, 0.0)?;
            let unit = if n < 0.0 {
                None
            } else {
                units.get(n as usize).copied()
            };
            Ok(if method == "charCodeAt" {
                JsValue::Number(unit.map_or(f64::NAN, |u| u as f64))
            } else {
                JsValue::string(unit.map_or_else(String::new, |u| String::from_utf16_lossy(&[u])))
            })
        }
        "slice" | "substring" | "substr" => {
            let start = num_default(vm, args, 0, 0.0)?;
            let mut a = if method == "substring" {
                start.max(0.0) as usize
            } else {
                index(start, len)
            }
            .min(len);
            let mut b = if method == "substr" {
                a.saturating_add(num_default(vm, args, 1, (len - a) as f64)?.max(0.0) as usize)
                    .min(len)
            } else if method == "substring" {
                num_default(vm, args, 1, len as f64)?.max(0.0) as usize
            } else {
                index(num_default(vm, args, 1, len as f64)?, len)
            }
            .min(len);
            if method == "substring" && a > b {
                std::mem::swap(&mut a, &mut b);
            }
            Ok(JsValue::string(String::from_utf16_lossy(
                &units[a..b.max(a)],
            )))
        }
        "trim" => Ok(JsValue::string(s.trim())),
        "toLowerCase" => Ok(JsValue::string(s.to_lowercase())),
        "toUpperCase" => Ok(JsValue::string(s.to_uppercase())),
        "concat" => {
            let mut out = s;
            for value in args {
                let part = vm.to_string(value.clone())?;
                vm.check_string_bytes(out.len().saturating_add(part.len()))?;
                out.push_str(&part);
            }
            if out.len() > vm.limits.max_string_bytes {
                return Err(JsError::new(ErrorKind::MemoryLimit, "string limit"));
            }
            Ok(JsValue::string(out))
        }
        "repeat" => {
            let n = num(vm, args, 0)?;
            if !n.is_finite() || n < 0.0 {
                return Err(JsError::new(ErrorKind::RangeError, "repeat limit"));
            }
            vm.check_string_bytes(s.len().saturating_mul(n as usize))?;
            Ok(JsValue::string(s.repeat(n as usize)))
        }
        "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith" => {
            let search = text(vm, args, 0)?;
            let from = (num_default(
                vm,
                args,
                1,
                if method == "lastIndexOf" {
                    len as f64
                } else {
                    0.0
                },
            )?
            .max(0.0) as usize)
                .min(len);
            let needle = search.encode_utf16().collect::<Vec<_>>();
            if method == "startsWith" {
                return Ok(JsValue::Bool(
                    units.get(from..from.saturating_add(needle.len())) == Some(needle.as_slice()),
                ));
            }
            if method == "endsWith" {
                let end = (num_default(vm, args, 1, len as f64)?.max(0.0) as usize).min(len);
                return Ok(JsValue::Bool(
                    end >= needle.len()
                        && units.get(end - needle.len()..end) == Some(needle.as_slice()),
                ));
            }
            let mut found = None;
            if needle.len() <= len {
                let positions: Box<dyn Iterator<Item = usize>> = if method == "lastIndexOf" {
                    Box::new((0..=from.min(len - needle.len())).rev())
                } else {
                    Box::new(from..=len - needle.len())
                };
                for i in positions {
                    vm.tick()?;
                    if units[i..i + needle.len()] == needle {
                        found = Some(i);
                        break;
                    }
                }
            }
            Ok(match method {
                "includes" => JsValue::Bool(found.is_some()),
                _ => JsValue::Number(found.map_or(-1.0, |i| i as f64)),
            })
        }
        "split" => {
            if args.is_empty() || arg(args, 0) == JsValue::Undefined {
                return vm.array(vec![JsValue::string(s)]);
            }
            let limit = to_i32(num_default(vm, args, 1, u32::MAX as f64)?) as u32 as usize;
            if let Some(id) = arg(args, 0).object() {
                if let ObjectKind::RegExp { regex, .. } = vm.heap.get(id)?.kind.clone() {
                    let values = collect_strings(vm, regex.split(&s).take(limit))?;
                    return vm.array(values);
                }
            }
            let separator = text(vm, args, 0)?;
            let values = if separator.is_empty() {
                collect_strings(
                    vm,
                    units
                        .into_iter()
                        .take(limit)
                        .map(|u| String::from_utf16_lossy(&[u])),
                )?
            } else {
                collect_strings(vm, s.split(&separator).take(limit))?
            };
            vm.array(values)
        }
        "match" | "search" => {
            let pattern = arg(args, 0);
            let regexp = if pattern.object().is_some_and(|id| {
                vm.heap
                    .get(id)
                    .is_ok_and(|o| matches!(o.kind, ObjectKind::RegExp { .. }))
            }) {
                pattern
            } else {
                let pattern = vm.to_string(pattern)?;
                vm.regexp(&pattern, "")?
            };
            let id = regexp.object().ok_or_else(|| error("regexp"))?;
            let ObjectKind::RegExp { regex, flags, .. } = vm.heap.get(id)?.kind.clone() else {
                return Err(error("regexp"));
            };
            if method == "search" {
                return Ok(JsValue::Number(
                    regex
                        .find(&s)
                        .map_or(-1.0, |m| s[..m.start()].encode_utf16().count() as f64),
                ));
            }
            if flags.contains('g') {
                let values = collect_strings(vm, regex.find_iter(&s).map(|m| m.as_str()))?;
                if values.is_empty() {
                    Ok(JsValue::Null)
                } else {
                    vm.array(values)
                }
            } else {
                regexp_call(vm, "exec", regexp, &[JsValue::string(s)])
            }
        }
        "replace" => {
            let pattern = arg(args, 0);
            let replacement = arg(args, 1);
            let matches = if let Some(id) = pattern.object() {
                if let ObjectKind::RegExp { regex, flags, .. } = vm.heap.get(id)?.kind.clone() {
                    let mut matches = Vec::new();
                    let mut bytes = 0usize;
                    for captures in regex.captures_iter(&s).take(if flags.contains('g') {
                        usize::MAX
                    } else {
                        1
                    }) {
                        vm.tick()?;
                        let Some(m) = captures.get(0) else {
                            continue;
                        };
                        let mut groups = Vec::new();
                        for capture in captures.iter() {
                            bytes = bytes
                                .saturating_add(128)
                                .saturating_add(capture.map_or(0, |m| m.len()));
                            vm.check_temporary(bytes)?;
                            groups.push(
                                capture.map_or(JsValue::Undefined, |m| JsValue::string(m.as_str())),
                            );
                        }
                        matches.push((m.start(), m.end(), groups));
                    }
                    matches
                } else {
                    Vec::new()
                }
            } else {
                let pattern = vm.to_string(pattern)?;
                s.find(&pattern)
                    .map(|i| vec![(i, i + pattern.len(), vec![JsValue::string(pattern)])])
                    .unwrap_or_default()
            };
            let mut out = String::new();
            let mut previous = 0;
            for (start, end, mut groups) in matches {
                append_bounded(vm, &mut out, &s[previous..start])?;
                let text = if vm.callable(&replacement) {
                    groups.push(JsValue::Number(s[..start].encode_utf16().count() as f64));
                    groups.push(JsValue::string(&s));
                    let v = vm.call_inner(
                        replacement.clone(),
                        JsValue::Undefined,
                        groups,
                        false,
                        None,
                    )?;
                    vm.to_string(v)?
                } else {
                    let template = vm.to_string(replacement.clone())?;
                    substitution(vm, &template, &s, start, end, &groups)?
                };
                append_bounded(vm, &mut out, &text)?;
                previous = end;
                if out.len() > vm.limits.max_string_bytes {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "replace string limit"));
                }
            }
            append_bounded(vm, &mut out, &s[previous..])?;
            Ok(JsValue::string(out))
        }
        _ => Err(error("unknown String method")),
    }
}
fn regexp_call(
    vm: &mut Vm,
    method: &str,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let id = this.object().ok_or_else(|| error("regexp receiver"))?;
    let ObjectKind::RegExp {
        regex,
        source,
        flags,
    } = vm.heap.get(id)?.kind.clone()
    else {
        return Err(error("regexp receiver"));
    };
    if method == "toString" {
        return Ok(JsValue::string(format!("/{source}/{flags}")));
    }
    let s = text(vm, args, 0)?;
    let indexed = flags.contains('g') || flags.contains('y');
    let start = if indexed {
        let last = vm.get(this.clone(), "lastIndex")?;
        let n = vm.to_number(last)?.max(0.0) as usize;
        utf16_byte(&s, n)
    } else {
        Some(0)
    };
    let captures = start.and_then(|start| regex.captures_at(&s, start));
    if let Some(captures) = captures {
        if let Some(m) = captures.get(0) {
            if flags.contains('y') && Some(m.start()) != start {
                vm.set(this, "lastIndex", JsValue::Number(0.0))?;
                return Ok(if method == "test" {
                    JsValue::Bool(false)
                } else {
                    JsValue::Null
                });
            }
            if indexed {
                vm.set(
                    this.clone(),
                    "lastIndex",
                    JsValue::Number(s[..m.end()].encode_utf16().count() as f64),
                )?;
            }
            if method == "test" {
                return Ok(JsValue::Bool(true));
            }
            let result = vm.array(
                captures
                    .iter()
                    .map(|m| m.map_or(JsValue::Undefined, |m| JsValue::string(m.as_str())))
                    .collect(),
            )?;
            vm.set(
                result.clone(),
                "index",
                JsValue::Number(s[..m.start()].encode_utf16().count() as f64),
            )?;
            vm.set(result.clone(), "input", JsValue::string(s))?;
            return Ok(result);
        }
    }
    if indexed {
        vm.set(this, "lastIndex", JsValue::Number(0.0))?;
    }
    Ok(if method == "test" {
        JsValue::Bool(false)
    } else {
        JsValue::Null
    })
}
fn utf16_byte(s: &str, target: usize) -> Option<usize> {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= target {
            return Some(i);
        }
        n += c.len_utf16();
    }
    if n == target {
        Some(s.len())
    } else {
        None
    }
}
fn math_call(vm: &mut Vm, name: &str, args: &[JsValue]) -> Result<JsValue, JsError> {
    if name == "random" {
        use std::sync::atomic::{AtomicU64, Ordering};
        static STATE: AtomicU64 = AtomicU64::new(0);
        let mut old = STATE.load(Ordering::Relaxed);
        if old == 0 {
            old = now() as u64 | 1;
        }
        let mut x = old;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        STATE.store(x, Ordering::Relaxed);
        return Ok(JsValue::Number((x >> 11) as f64 / 9007199254740992.0));
    }
    if name == "min" || name == "max" {
        let mut result = if name == "min" {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
        for value in args {
            let n = vm.to_number(value.clone())?;
            if n.is_nan() {
                return Ok(JsValue::Number(f64::NAN));
            }
            result = if name == "min" {
                result.min(n)
            } else {
                result.max(n)
            };
        }
        return Ok(JsValue::Number(result));
    }
    let a = num(vm, args, 0)?;
    let n = match name {
        "abs" => a.abs(),
        "acos" => a.acos(),
        "asin" => a.asin(),
        "atan" => a.atan(),
        "atan2" => a.atan2(num(vm, args, 1)?),
        "ceil" => a.ceil(),
        "cos" => a.cos(),
        "exp" => a.exp(),
        "floor" => a.floor(),
        "log" => a.ln(),
        "log2" => a.log2(),
        "log10" => a.log10(),
        "pow" => a.powf(num(vm, args, 1)?),
        "round" => {
            if (-0.5..0.0).contains(&a) {
                -0.0
            } else {
                (a + 0.5).floor()
            }
        }
        "sin" => a.sin(),
        "sqrt" => a.sqrt(),
        "tan" => a.tan(),
        "trunc" => a.trunc(),
        "sign" => {
            if a == 0.0 || a.is_nan() {
                a
            } else {
                a.signum()
            }
        }
        _ => return Err(error("unknown Math method")),
    };
    Ok(JsValue::Number(n))
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}
fn clip_date(n: f64) -> f64 {
    if !n.is_finite() || n.abs() > 8.64e15 {
        f64::NAN
    } else {
        n.trunc()
    }
}
fn timestamp(n: f64) -> Option<time::OffsetDateTime> {
    if !n.is_finite() {
        return None;
    }
    time::OffsetDateTime::from_unix_timestamp_nanos((n * 1_000_000.0) as i128).ok()
}
fn date_string(n: f64) -> String {
    timestamp(n)
        .and_then(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "Invalid Date".into())
}
fn parse_date(s: &str) -> f64 {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map(|d| d.unix_timestamp_nanos() as f64 / 1_000_000.0)
        .or_else(|_| {
            time::Date::parse(s, time::macros::format_description!("[year]-[month]-[day]"))
                .map(|d| d.midnight().assume_utc().unix_timestamp_nanos() as f64 / 1_000_000.0)
        })
        .unwrap_or(f64::NAN)
}
fn date_components(vm: &mut Vm, args: &[JsValue]) -> Result<f64, JsError> {
    let mut components = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (i, component) in components.iter_mut().enumerate() {
        if let Some(value) = args.get(i) {
            *component = vm.to_number(value.clone())?;
        }
    }
    if components.iter().any(|n| !n.is_finite()) {
        return Ok(f64::NAN);
    }
    for component in &mut components {
        *component = component.trunc();
    }
    let mut year = components[0];
    let month = components[1];
    if (0.0..=99.0).contains(&year) {
        year += 1900.0;
    }
    year += month.div_euclid(12.0);
    if !(i32::MIN as f64..=i32::MAX as f64).contains(&year) {
        return Ok(f64::NAN);
    }
    let month = time::Month::try_from((month.rem_euclid(12.0) + 1.0) as u8).ok();
    let date = month.and_then(|m| time::Date::from_calendar_date(year as i32, m, 1).ok());
    let Some(date) = date else {
        return Ok(f64::NAN);
    };
    let day = components[2] - 1.0;
    let hours = components[3];
    let minutes = components[4];
    let seconds = components[5];
    let millis = components[6];
    Ok(
        date.midnight().assume_utc().unix_timestamp_nanos() as f64 / 1_000_000.0
            + day * 86400000.0
            + hours * 3600000.0
            + minutes * 60000.0
            + seconds * 1000.0
            + millis,
    )
}
fn date_call(
    vm: &mut Vm,
    method: &str,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let id = this.object().ok_or_else(|| error("Date receiver"))?;
    let ObjectKind::Date(n) = vm.heap.get(id)?.kind else {
        return Err(error("Date receiver"));
    };
    match method {
        "getTime" | "valueOf" => return Ok(JsValue::Number(n)),
        "setTime" => {
            let n = clip_date(num(vm, args, 0)?);
            vm.heap.get_mut(id)?.kind = ObjectKind::Date(n);
            return Ok(JsValue::Number(n));
        }
        "toString" => return Ok(JsValue::string(date_string(n))),
        "toJSON" if !n.is_finite() => return Ok(JsValue::Null),
        "toJSON" | "toISOString" => {
            let Some(d) = timestamp(n) else {
                return Err(JsError::new(ErrorKind::RangeError, "invalid date"));
            };
            let s = d
                .format(time::macros::format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
                ))
                .map_err(|e| error(&e.to_string()))?;
            return Ok(JsValue::string(s));
        }
        _ => {}
    }
    let Some(d) = timestamp(n) else {
        return Ok(JsValue::Number(f64::NAN));
    };
    let n = match method {
        "getFullYear" | "getUTCFullYear" => d.year() as f64,
        "getMonth" | "getUTCMonth" => d.month() as u8 as f64 - 1.0,
        "getDate" | "getUTCDate" => d.day() as f64,
        "getDay" | "getUTCDay" => d.weekday().number_days_from_sunday() as f64,
        "getHours" | "getUTCHours" => d.hour() as f64,
        "getMinutes" | "getUTCMinutes" => d.minute() as f64,
        "getSeconds" | "getUTCSeconds" => d.second() as f64,
        "getMilliseconds" | "getUTCMilliseconds" => d.millisecond() as f64,
        "getTimezoneOffset" => 0.0,
        _ => return Err(error("unknown Date method")),
    };
    Ok(JsValue::Number(n))
}
fn from_json(vm: &mut Vm, value: serde_json::Value, depth: usize) -> Result<JsValue, JsError> {
    vm.tick()?;
    if depth > 128 {
        return Err(JsError::new(ErrorKind::RangeError, "JSON depth"));
    }
    match value {
        serde_json::Value::Null => Ok(JsValue::Null),
        serde_json::Value::Bool(b) => Ok(JsValue::Bool(b)),
        serde_json::Value::Number(n) => Ok(JsValue::Number(n.as_f64().unwrap_or(f64::NAN))),
        serde_json::Value::String(s) => Ok(JsValue::string(s)),
        serde_json::Value::Array(v) => {
            let mut values = Vec::new();
            for v in v {
                values.push(from_json(vm, v, depth + 1)?);
            }
            vm.array(values)
        }
        serde_json::Value::Object(v) => {
            let object = vm.object()?;
            for (k, v) in v {
                let value = from_json(vm, v, depth + 1)?;
                vm.set(object.clone(), &k, value)?;
            }
            Ok(object)
        }
    }
}
fn to_json(
    vm: &mut Vm,
    value: JsValue,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
    bytes: &mut usize,
) -> Result<Option<serde_json::Value>, JsError> {
    vm.tick()?;
    *bytes = bytes.saturating_add(32);
    if let JsValue::String(s) = &value {
        *bytes = bytes.saturating_add(s.len().saturating_mul(6));
    }
    vm.check_string_bytes(*bytes)?;
    if depth > 128 {
        return Err(JsError::new(ErrorKind::RangeError, "JSON depth"));
    }
    Ok(Some(match value {
        JsValue::Undefined | JsValue::Symbol(..) => return Ok(None),
        JsValue::Null => serde_json::Value::Null,
        JsValue::Bool(b) => b.into(),
        JsValue::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                serde_json::Value::Number(serde_json::Number::from(n as i64))
            } else {
                serde_json::Number::from_f64(n)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            }
        }
        JsValue::String(s) => serde_json::Value::String(s.to_string()),
        JsValue::Object(id) => {
            if vm.callable(&JsValue::Object(id)) {
                return Ok(None);
            }
            if !seen.insert(id) {
                return Err(error("cyclic JSON value"));
            }
            let kind = vm.heap.get(id)?.kind.clone();
            let value = match kind {
                ObjectKind::Array => {
                    let mut out = Vec::new();
                    for value in vm.array_values(JsValue::Object(id))? {
                        out.push(
                            to_json(vm, value, seen, depth + 1, bytes)?
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                    serde_json::Value::Array(out)
                }
                ObjectKind::Date(n) => {
                    if n.is_finite() {
                        serde_json::Value::String(date_string(n))
                    } else {
                        serde_json::Value::Null
                    }
                }
                ObjectKind::Boxed(v) => {
                    to_json(vm, v, seen, depth + 1, bytes)?.unwrap_or(serde_json::Value::Null)
                }
                _ => {
                    let mut out = serde_json::Map::new();
                    for key in vm.keys(&JsValue::Object(id), true)? {
                        if let PropertyKey::String(k) = key {
                            *bytes = bytes.saturating_add(k.len().saturating_mul(6));
                            let value = vm.get(JsValue::Object(id), &k)?;
                            if let Some(value) = to_json(vm, value, seen, depth + 1, bytes)? {
                                out.insert(k, value);
                            }
                        }
                    }
                    serde_json::Value::Object(out)
                }
            };
            seen.remove(&id);
            value
        }
    }))
}
