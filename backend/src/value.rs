//! 值编码：QSDB wire 值 → 契约 JSON。
//!
//! - NULL → `null`；布尔 → boolean
//! - 整数 → number；超出 JS 安全整数（±2^53-1）→ 十进制字符串
//! - 浮点 → number；NaN/±Infinity → `"NaN"` / `"Infinity"` / `"-Infinity"`
//! - Decimal → 精确十进制字符串；Binary → 小写 hex
//! - DATE/TIME/TIMESTAMP → 契约字符串（与 QSDB `format_date`/`format_time_ns`/
//!   `format_timestamp_ns` 逐字一致）；Interval → `"Nmo Nd Nns"`
//!
//! 日期/时间格式在本地实现（civil_from_days 等标准算法），不拉 chrono——
//! 与 QSDB 服务端的 naive-UTC 输出保持逐字一致即可。

use qsdb_client::{ColumnDesc, Tag, ValueRef};
use serde_json::{json, Value};

/// 2^53 - 1：JS `Number.MAX_SAFE_INTEGER`。
const JS_SAFE_MAX: i64 = 9007199254740991;

/// 把一个 cell 编码成契约 JSON。
pub fn encode_cell(v: ValueRef<'_>, col: &ColumnDesc) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Bool(b) => json!(b),
        ValueRef::I8(x) => json!(i64::from(x)),
        ValueRef::I16(x) => json!(i64::from(x)),
        ValueRef::I32(x) => json!(i64::from(x)),
        ValueRef::I64(x) => {
            if x.unsigned_abs() > JS_SAFE_MAX as u64 {
                // 超安全整数：保精度走字符串
                json!(x.to_string())
            } else {
                json!(x)
            }
        }
        ValueRef::F32(x) => encode_f64(f64::from(x)),
        ValueRef::F64(x) => encode_f64(x),
        ValueRef::Utf8(s) => json!(s),
        ValueRef::Binary(b) => {
            let mut hex = String::with_capacity(b.len() * 2);
            for byte in b.iter() {
                hex.push_str(&format!("{byte:02x}"));
            }
            json!(hex)
        }
        ValueRef::Date32(days) => json!(format_date(days)),
        ValueRef::Time64Ns(ns) => json!(format_time_ns(ns)),
        ValueRef::TimestampNs(ns) => json!(format_timestamp_ns(ns)),
        ValueRef::Decimal128(unscaled) => json!(format_decimal(unscaled, col.decimal.map(|(_, scale)| scale).unwrap_or(0))),
        ValueRef::Interval { months, days, nanos } => {
            json!(format!("{months}mo {days}d {nanos}ns"))
        }
    }
}

fn encode_f64(x: f64) -> Value {
    if x.is_nan() {
        json!("NaN")
    } else if x == f64::INFINITY {
        json!("Infinity")
    } else if x == f64::NEG_INFINITY {
        json!("-Infinity")
    } else {
        json!(x)
    }
}

/// 未缩放 i128 + scale → 精确十进制字符串（无精度丢失）。
pub fn format_decimal(unscaled: i128, scale: i8) -> String {
    if scale <= 0 {
        // scale ≤ 0：右补零（10^-负数 = 整数放大）
        let mut s = unscaled.to_string();
        for _ in 0..(-scale) {
            s.push('0');
        }
        return s;
    }
    let scale = scale as u32;
    let neg = unscaled < 0;
    let digits = unscaled.unsigned_abs().to_string();
    let (int_part, frac_part) = if digits.len() > scale as usize {
        let split = digits.len() - scale as usize;
        (digits[..split].to_string(), digits[split..].to_string())
    } else {
        ("0".to_string(), format!("{:0>width$}", digits, width = scale as usize))
    };
    let frac = frac_part.trim_end_matches('0');
    let body = if frac.is_empty() {
        int_part
    } else {
        format!("{int_part}.{frac}")
    };
    if neg && unscaled != 0 {
        format!("-{body}")
    } else {
        body
    }
}

// ---------------------------------------------------------------------------
// 时间格式（与 QSDB types/temporal.rs 的输出逐字一致）
// ---------------------------------------------------------------------------

const NANOS_PER_SEC: i64 = 1_000_000_000;
const NANOS_PER_MIN: i64 = 60 * NANOS_PER_SEC;
const NANOS_PER_HOUR: i64 = 60 * NANOS_PER_MIN;
const NANOS_PER_DAY: i64 = 24 * NANOS_PER_HOUR;

/// days since epoch → `YYYY-MM-DD`（Howard Hinnant civil_from_days）。
pub fn format_date(days: i32) -> String {
    let (y, m, d) = civil_from_days(i64::from(days));
    format!("{y:04}-{m:02}-{d:02}")
}

/// ns since midnight → `HH:MM:SS[.fff]`（超范围按一天取模；分数按 3/6/9 位
/// 分组省略尾零，与 QSDB `format_frac` 一致）。
pub fn format_time_ns(ns: i64) -> String {
    let ns = ns.rem_euclid(NANOS_PER_DAY);
    let h = ns / NANOS_PER_HOUR;
    let m = (ns % NANOS_PER_HOUR) / NANOS_PER_MIN;
    let sec = (ns % NANOS_PER_MIN) / NANOS_PER_SEC;
    let frac = ns % NANOS_PER_SEC;
    format!("{h:02}:{m:02}:{sec:02}{}", format_frac(frac))
}

/// ns since epoch → `YYYY-MM-DD HH:MM:SS[.fff]`（naive UTC）。
pub fn format_timestamp_ns(ns: i64) -> String {
    let days = ns.div_euclid(NANOS_PER_DAY);
    let rem = ns.rem_euclid(NANOS_PER_DAY);
    let h = rem / NANOS_PER_HOUR;
    let m = (rem % NANOS_PER_HOUR) / NANOS_PER_MIN;
    let sec = (rem % NANOS_PER_MIN) / NANOS_PER_SEC;
    let frac = rem % NANOS_PER_SEC;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{sec:02}{}", format_frac(frac))
}

/// 与 QSDB `format_frac` 逐字一致：0 → 空；毫秒精度 3 位、微秒精度 6 位、
/// 其余 9 位（分组内不再去尾零）。
fn format_frac(nanos: i64) -> String {
    if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{nanos:09}")
    }
}

/// Howard Hinnant `civil_from_days`（proleptic Gregorian）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 列描述 → 契约列 JSON：`{name, tag, typeName, nullable}`。
pub fn column_json(desc: &ColumnDesc) -> Value {
    let tag_code = match desc.tag {
        Tag::Bool => "bool",
        Tag::I8 => "i8",
        Tag::I16 => "i16",
        Tag::I32 => "i32",
        Tag::I64 => "i64",
        Tag::F32 => "f32",
        Tag::F64 => "f64",
        Tag::Utf8 => "utf8",
        Tag::Binary => "binary",
        Tag::Date32 => "date32",
        Tag::Time64Ns => "time64_ns",
        Tag::TimestampNs => "timestamp_ns",
        Tag::Decimal128 => "decimal128",
        Tag::Interval => "interval",
    };
    // typeName：标签人读名 + 列级元数据修饰（信息展示用，不做兼容承诺）。
    let mut type_name = desc.tag.name().to_string();
    match desc.tag {
        Tag::Utf8 if desc.max_chars.is_some_and(|n| n > 0) => {
            type_name = format!("VARCHAR({})", desc.max_chars.unwrap());
        }
        Tag::Decimal128 => {
            if let Some((p, s)) = desc.decimal {
                type_name = format!("DECIMAL({p},{s})");
            }
        }
        Tag::TimestampNs if desc.tz.is_some() => {
            type_name = format!("TIMESTAMP({})", desc.tz.clone().unwrap());
        }
        _ => {}
    }
    json!({
        "name": desc.name,
        "tag": tag_code,
        "typeName": type_name,
        "nullable": desc.nullable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsdb_client::ColumnDesc;

    fn desc(tag: Tag) -> ColumnDesc {
        ColumnDesc::new("c", tag)
    }

    #[test]
    fn integers_within_safe_range_are_numbers() {
        assert_eq!(encode_cell(ValueRef::I64(123), &desc(Tag::I64)), json!(123));
        assert_eq!(
            encode_cell(ValueRef::I64(JS_SAFE_MAX), &desc(Tag::I64)),
            json!(JS_SAFE_MAX)
        );
    }

    #[test]
    fn integers_beyond_safe_range_become_strings() {
        assert_eq!(
            encode_cell(ValueRef::I64(JS_SAFE_MAX + 1), &desc(Tag::I64)),
            json!("9007199254740992")
        );
        assert_eq!(
            encode_cell(ValueRef::I64(i64::MIN), &desc(Tag::I64)),
            json!("-9223372036854775808")
        );
    }

    #[test]
    fn floats_special_values() {
        assert_eq!(encode_cell(ValueRef::F64(f64::NAN), &desc(Tag::F64)), json!("NaN"));
        assert_eq!(
            encode_cell(ValueRef::F64(f64::INFINITY), &desc(Tag::F64)),
            json!("Infinity")
        );
        assert_eq!(
            encode_cell(ValueRef::F32(f32::NEG_INFINITY), &desc(Tag::F32)),
            json!("-Infinity")
        );
        assert_eq!(encode_cell(ValueRef::F64(1.5), &desc(Tag::F64)), json!(1.5));
    }

    #[test]
    fn binary_is_lowercase_hex() {
        assert_eq!(
            encode_cell(ValueRef::Binary(&[0xde, 0xad, 0xBe]), &desc(Tag::Binary)),
            json!("deadbe")
        );
    }

    #[test]
    fn temporal_formats_match_qsdb() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(19_722), "2023-12-31"); // 抽查（19723 = 2024-01-01）
        assert_eq!(format_date(19_723), "2024-01-01");
        assert_eq!(format_date(-1), "1969-12-31");
        assert_eq!(format_time_ns(0), "00:00:00");
        assert_eq!(format_time_ns(NANOS_PER_SEC + 1), "00:00:01.000000001");
        assert_eq!(format_time_ns((6 * 3600 + 30 * 60) * NANOS_PER_SEC), "06:30:00");
        // QSDB format_frac 分组：毫秒 3 位、微秒 6 位、纳秒 9 位
        assert_eq!(format_time_ns(NANOS_PER_SEC + 1_000_000), "00:00:01.001");
        assert_eq!(format_time_ns(NANOS_PER_SEC + 1000), "00:00:01.000001");
        assert_eq!(format_time_ns(NANOS_PER_SEC + 123), "00:00:01.000000123");
        assert_eq!(format_time_ns(NANOS_PER_SEC + 100_000_000), "00:00:01.100");
        // 2000-01-01T12:00:00.5 UTC = 946728000500000000
        assert_eq!(format_timestamp_ns(946_728_000_500_000_000), "2000-01-01 12:00:00.500");
        assert_eq!(format_timestamp_ns(0), "1970-01-01 00:00:00");
        // 负时间（1969-12-31）
        assert_eq!(format_timestamp_ns(-1), "1969-12-31 23:59:59.999999999");
    }

    #[test]
    fn decimal_scaling() {
        assert_eq!(format_decimal(12345, 2), "123.45");
        assert_eq!(format_decimal(-12345, 2), "-123.45");
        assert_eq!(format_decimal(5, 3), "0.005");
        assert_eq!(format_decimal(100, 0), "100");
        assert_eq!(format_decimal(100, -2), "10000");
        assert_eq!(format_decimal(1200, 2), "12"); // 尾零省略
    }

    #[test]
    fn interval_format() {
        assert_eq!(
            encode_cell(
                ValueRef::Interval { months: 1, days: 2, nanos: 3 },
                &desc(Tag::Interval)
            ),
            json!("1mo 2d 3ns")
        );
    }

    #[test]
    fn column_json_shape() {
        let mut d = ColumnDesc::new("amount", Tag::Decimal128);
        d.nullable = true;
        d.decimal = Some((10, 2));
        let v = column_json(&d);
        assert_eq!(v["name"], "amount");
        assert_eq!(v["tag"], "decimal128");
        assert_eq!(v["typeName"], "DECIMAL(10,2)");
        assert_eq!(v["nullable"], true);
    }
}
