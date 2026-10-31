//! Explicit native byte-length/precision semantics, independent of fonts/VMs.
use crysirror_text::format::{format_one_bytes,ByteFormatValue,MAX_BYTE_FORMAT_BYTES};
#[test]
fn string_zero_padding_precision_and_embedded_nul_follow_declared_byte_length() {
    // format.c FMT_STRING ignores precision/zero; libsys4 string_append copies
    // val->size bytes with memcpy, not strlen. The template itself has no NUL.
    let result=format_one_bytes(b"[%05.1s]",ByteFormatValue::Bytes(b"a\0b")).unwrap();
    assert!(result.matched);assert_eq!(result.bytes,b"[  a\0b]");
    let result=format_one_bytes(b"%010.1d",ByteFormatValue::Integer(-7)).unwrap();
    assert_eq!(result.bytes,b"-000000007");
}
#[test]
fn unused_byte_value_is_not_truncated_or_eagerly_formatted() {
    let large=vec![b'x';MAX_BYTE_FORMAT_BYTES+1];
    let output=format_one_bytes(b"unchanged%d",ByteFormatValue::Bytes(&large)).unwrap();
    assert!(!output.matched);assert_eq!(output.bytes,b"unchanged%d");
    assert!(format_one_bytes(b"%s",ByteFormatValue::Bytes(&large)).is_err());
}
