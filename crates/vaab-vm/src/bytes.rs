//! Bytes constructors and little-endian reads.
//!
//! Out-of-range reads become `failure BytesError`, so a bad offset is a
//! value the program can match rather than a stop.

use base64::Engine;

use crate::bytecode::{BytesErrors, BytesOp, Program};
use crate::error::Fault;
use crate::value::{Ref, Value, Variant};

pub fn apply(
    op: BytesOp,
    errors: BytesErrors,
    program: &Program,
    stack: &mut Vec<Value>,
) -> Result<(), Fault> {
    let result = match op {
        BytesOp::FromBase64 | BytesOp::FromHex | BytesOp::FromBase58 => {
            let text = pop_text(stack, "a Bytes constructor")?;
            match decode(op, &text) {
                Ok(bytes) => Value::success(Value::bytes(bytes)),
                Err(message) => invalid(program, errors, message)?,
            }
        }
        BytesOp::Slice => {
            let to = pop_int(stack, "Bytes.slice")?;
            let from = pop_int(stack, "Bytes.slice")?;
            let raw = pop_bytes(stack, "Bytes.slice")?;
            match slice_range(from, to, raw.len()) {
                Ok(range) => Value::success(Value::bytes(raw[range].to_vec())),
                Err(at) => out_of_range(program, errors, at, raw.len())?,
            }
        }
        BytesOp::U8 => read_int(stack, program, errors, 1, |slice| Ok(i64::from(slice[0])))?,
        BytesOp::U32Le => read_int(stack, program, errors, 4, |slice| {
            Ok(i64::from(u32::from_le_bytes(slice.try_into().expect("4 bytes"))))
        })?,
        BytesOp::U64Le => read_int(stack, program, errors, 8, |slice| {
            let value = u64::from_le_bytes(slice.try_into().expect("8 bytes"));
            i64::try_from(value).map_err(|_| ReadFail::TooLarge)
        })?,
        BytesOp::I64Le => read_int(stack, program, errors, 8, |slice| {
            Ok(i64::from_le_bytes(slice.try_into().expect("8 bytes")))
        })?,
        BytesOp::Bool => {
            let at = pop_int(stack, "Bytes.bool")?;
            let raw = pop_bytes(stack, "Bytes.bool")?;
            match offset(at, 1, raw.len()) {
                Ok(start) => Value::success(Value::Bool(raw[start] != 0)),
                Err(at) => out_of_range(program, errors, at, raw.len())?,
            }
        }
        BytesOp::ToHex => {
            let raw = pop_bytes(stack, "Bytes.to_hex")?;
            Value::text(hex::encode(raw))
        }
        BytesOp::ToBase58 => {
            let raw = pop_bytes(stack, "Bytes.to_base58")?;
            Value::text(bs58::encode(raw).into_string())
        }
        BytesOp::ToBase64 => {
            let raw = pop_bytes(stack, "Bytes.to_base64")?;
            Value::text(base64::engine::general_purpose::STANDARD.encode(raw))
        }
        BytesOp::StartsWith => {
            let prefix = pop_bytes(stack, "Bytes.starts_with")?;
            let raw = pop_bytes(stack, "Bytes.starts_with")?;
            Value::Bool(raw.starts_with(prefix.as_ref()))
        }
        BytesOp::BorshText => {
            let at = pop_int(stack, "Bytes.borsh_text")?;
            let raw = pop_bytes(stack, "Bytes.borsh_text")?;
            match borsh_text(&raw, at) {
                Ok((text, next)) => {
                    Value::success(Value::Tuple(Ref::new(vec![Value::text(text), Value::Int(next)])))
                }
                Err(ReadFail::OutOfRange(at)) => out_of_range(program, errors, at, raw.len())?,
                Err(ReadFail::TooLarge) => too_large(program, errors, at)?,
                Err(ReadFail::Invalid(message)) => invalid(program, errors, message)?,
            }
        }
    };
    stack.push(result);
    Ok(())
}

enum ReadFail {
    OutOfRange(i64),
    TooLarge,
    Invalid(String),
}

fn read_int(
    stack: &mut Vec<Value>,
    program: &Program,
    errors: BytesErrors,
    need: usize,
    decode: impl FnOnce(&[u8]) -> Result<i64, ReadFail>,
) -> Result<Value, Fault> {
    let at = pop_int(stack, "a Bytes integer read")?;
    let raw = pop_bytes(stack, "a Bytes integer read")?;
    match offset(at, need, raw.len()) {
        Ok(start) => match decode(&raw[start..start + need]) {
            Ok(number) => Ok(Value::success(Value::Int(number))),
            Err(ReadFail::TooLarge) => too_large(program, errors, at),
            Err(ReadFail::OutOfRange(at)) => out_of_range(program, errors, at, raw.len()),
            Err(ReadFail::Invalid(message)) => invalid(program, errors, message),
        },
        Err(at) => out_of_range(program, errors, at, raw.len()),
    }
}

fn decode(op: BytesOp, text: &str) -> Result<Vec<u8>, String> {
    match op {
        BytesOp::FromBase64 => decode_base64(text),
        BytesOp::FromHex => decode_hex(text),
        BytesOp::FromBase58 => {
            bs58::decode(text.trim()).into_vec().map_err(|error| format!("not base58: {error}"))
        }
        _ => Err("not a Bytes constructor".to_string()),
    }
}

fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let trimmed = text.trim();
    base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .map_err(|error| format!("not base64: {error}"))
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    let trimmed = text.trim();
    let stripped = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);
    let compact: String = stripped.chars().filter(|letter| !letter.is_whitespace()).collect();
    hex::decode(compact).map_err(|error| format!("not hex: {error}"))
}

fn offset(at: i64, need: usize, length: usize) -> Result<usize, i64> {
    if at < 0 {
        return Err(at);
    }
    let start = at as u64;
    if start > usize::MAX as u64 {
        return Err(at);
    }
    let start = start as usize;
    if start.checked_add(need).map_or(true, |end| end > length) {
        return Err(at);
    }
    Ok(start)
}

fn slice_range(from: i64, to: i64, length: usize) -> Result<std::ops::Range<usize>, i64> {
    if from < 0 {
        return Err(from);
    }
    if to < from {
        return Err(from);
    }
    if from as u64 > usize::MAX as u64 {
        return Err(from);
    }
    if to as u64 > usize::MAX as u64 {
        return Err(to);
    }
    let start = from as usize;
    let end = to as usize;
    if start > length {
        return Err(from);
    }
    if end > length {
        return Err(to);
    }
    Ok(start..end)
}

fn borsh_text(raw: &[u8], at: i64) -> Result<(String, i64), ReadFail> {
    let start = offset(at, 4, raw.len()).map_err(ReadFail::OutOfRange)?;
    let len = u32::from_le_bytes(raw[start..start + 4].try_into().expect("4 bytes")) as usize;
    let body = offset(at + 4, len, raw.len()).map_err(ReadFail::OutOfRange)?;
    let text = std::str::from_utf8(&raw[body..body + len])
        .map_err(|_| ReadFail::Invalid("not utf-8".to_string()))?;
    let next = i64::try_from(body + len).map_err(|_| ReadFail::OutOfRange(at))?;
    Ok((text.to_string(), next))
}

fn pop_text<'a>(stack: &'a mut Vec<Value>, what: &str) -> Result<String, Fault> {
    match stack.pop() {
        Some(Value::Text(text)) => Ok(text.to_string()),
        Some(_) => Err(Fault::Confused(confused(what))),
        None => Err(Fault::Confused(confused(what))),
    }
}

fn pop_int(stack: &mut Vec<Value>, what: &str) -> Result<i64, Fault> {
    match stack.pop() {
        Some(Value::Int(number)) => Ok(number),
        _ => Err(Fault::Confused(confused(what))),
    }
}

fn pop_bytes(stack: &mut Vec<Value>, what: &str) -> Result<Ref<[u8]>, Fault> {
    match stack.pop() {
        Some(Value::Bytes(bytes)) => Ok(bytes),
        _ => Err(Fault::Confused(confused(what))),
    }
}

fn confused(what: &str) -> &'static str {
    match what {
        "a Bytes constructor" => "a Bytes constructor expected text",
        "Bytes.slice" => "Bytes.slice expected Bytes and two Ints",
        "Bytes.bool" => "Bytes.bool expected Bytes and an Int",
        "Bytes.to_hex" => "Bytes.to_hex expected Bytes",
        "Bytes.to_base58" => "Bytes.to_base58 expected Bytes",
        "Bytes.to_base64" => "Bytes.to_base64 expected Bytes",
        "Bytes.starts_with" => "Bytes.starts_with expected two Bytes",
        "Bytes.borsh_text" => "Bytes.borsh_text expected Bytes and an Int",
        "a Bytes integer read" => "a Bytes integer read expected Bytes and an Int",
        _ => "a Bytes operation was given a value of the wrong shape",
    }
}

fn out_of_range(
    program: &Program,
    errors: BytesErrors,
    at: i64,
    length: usize,
) -> Result<Value, Fault> {
    fail(
        program,
        errors.out_of_range,
        vec![Value::Int(at), Value::Int(length as i64)],
    )
}

fn too_large(program: &Program, errors: BytesErrors, at: i64) -> Result<Value, Fault> {
    fail(program, errors.too_large, vec![Value::Int(at)])
}

fn invalid(program: &Program, errors: BytesErrors, message: String) -> Result<Value, Fault> {
    fail(program, errors.invalid, vec![Value::text(message)])
}

fn fail(program: &Program, layout: u32, fields: Vec<Value>) -> Result<Value, Fault> {
    let layout = program.variants.get(layout as usize).cloned().ok_or(Fault::Confused(
        "BytesError named a variant that is not there",
    ))?;
    Ok(Value::failure(Value::Variant(Ref::new(Variant { layout, fields }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decodes_with_or_without_a_prefix() {
        assert_eq!(decode_hex("bddb").unwrap(), vec![0xbd, 0xdb]);
        assert_eq!(decode_hex("0xBDdb").unwrap(), vec![0xbd, 0xdb]);
        assert!(decode_hex("xyz").is_err());
    }

    #[test]
    fn a_u64_above_i64_max_is_too_large() {
        let bytes = u64::MAX.to_le_bytes();
        assert!(matches!(
            i64::try_from(u64::from_le_bytes(bytes)).map_err(|_| ReadFail::TooLarge),
            Err(ReadFail::TooLarge)
        ));
    }

    #[test]
    fn starts_with_is_false_when_the_prefix_is_longer() {
        assert!(!b"ab".starts_with(b"abc"));
        assert!(b"abc".starts_with(b"ab"));
    }
}
