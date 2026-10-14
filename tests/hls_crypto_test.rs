use easydownvideo::core::hls::crypto::decrypt_aes128;

const KEY: [u8; 16] = *b"\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff";
const IV: [u8; 16] = *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10";

// 向量 A：带 PKCS7 填充的密文（32 字节，16 字节明文 + 16 字节填充），
// 来自 `openssl enc -aes-128-cbc -K 00112233445566778899aabbccddeeff -iv 0102030405060708090a0b0c0d0e0f10`
const CT_PADDED_HEX: &str = "1e527b1499830859517e100905fda90f2e388989269325181edc799208c0e26b";
// 向量 B：无填充的密文（16 字节，-nopad），用于走 PKCS7 去填充失败的回退分支
const CT_UNPADDED_HEX: &str = "1e527b1499830859517e100905fda90f";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[test]
fn decrypts_pkcs7_padded_segment() {
    let ct = unhex(CT_PADDED_HEX);
    assert_eq!(
        ct.len(),
        32,
        "向量 A 应为 32 字节（16 字节明文 + 16 字节 PKCS7 填充）"
    );
    let pt = decrypt_aes128(&KEY, &IV, &ct).unwrap();
    assert_eq!(pt, b"0123456789abcdef");
    assert_eq!(pt.len(), 16, "PKCS7 填充应被去除");
}

#[test]
fn falls_back_to_nopadding_when_pkcs7_unpad_fails() {
    // 末尾字节 0x66 > 16，不是合法 PKCS7 填充 → 应回退到 NoPadding 并原样返回 16 字节
    let ct = unhex(CT_UNPADDED_HEX);
    assert_eq!(ct.len(), 16);
    let pt = decrypt_aes128(&KEY, &IV, &ct).unwrap();
    assert_eq!(pt, b"0123456789abcdef");
}

#[test]
fn rejects_non_blocksize_input() {
    assert!(decrypt_aes128(&KEY, &IV, b"not-a-multiple!").is_err());
}

#[test]
fn rejects_empty_input() {
    assert!(decrypt_aes128(&KEY, &IV, b"").is_err());
}
