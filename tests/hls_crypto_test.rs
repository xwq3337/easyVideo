use easydownvideo::core::hls::crypto::{decrypt_aes128, HlsCryptoError};

const KEY: [u8; 16] = *b"\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff";
const IV: [u8; 16] = *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10";

// 向量 A：带 PKCS7 填充的密文（32 字节，16 字节明文 + 16 字节填充），
// 来自 `openssl enc -aes-128-cbc -K 00112233445566778899aabbccddeeff -iv 0102030405060708090a0b0c0d0e0f10`
const CT_PADDED_HEX: &str = "1e527b1499830859517e100905fda90f2e388989269325181edc799208c0e26b";
// 向量 B：无填充的密文（16 字节，-nopad），末尾解出 0x66 不是合法 PKCS7 填充
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
fn rejects_bad_padding_instead_of_returning_garbage() {
    // 向量 B：无填充密文，末尾解出 0x66(>16)，不是合法 PKCS7 填充。
    // HLS AES-128 没有 MAC，PKCS7 校验是**唯一**的完整性信号——
    // 这里必须报错，绝不能"宽容地"把明文（连同填充）返回出去：
    // 那等于把每一次可检测的损坏变成静默的坏视频。
    let ct = unhex(CT_UNPADDED_HEX);
    assert_eq!(ct.len(), 16);
    let err = decrypt_aes128(&KEY, &IV, &ct).unwrap_err();
    assert!(
        matches!(err, HlsCryptoError::BadPadding),
        "期望 BadPadding，实际: {err}"
    );
}

#[test]
fn wrong_key_errors_rather_than_returning_garbage() {
    // 用错误密钥解出来的整段都是乱码；PKCS7 校验必须把它拦住。
    let ct = unhex(CT_PADDED_HEX);
    let wrong_key = [0u8; 16];
    let r = decrypt_aes128(&wrong_key, &IV, &ct);
    assert!(r.is_err(), "错误密钥必须报错，而不是产出看似成功的乱码");
}

#[test]
fn rejects_non_blocksize_input() {
    let err = decrypt_aes128(&KEY, &IV, b"not-a-multiple!").unwrap_err();
    assert!(
        matches!(err, HlsCryptoError::InvalidLength(15)),
        "实际: {err}"
    );
}

#[test]
fn rejects_empty_input() {
    let err = decrypt_aes128(&KEY, &IV, b"").unwrap_err();
    // 空输入必须走 Empty 而不是 InvalidLength：0 本身是 16 的倍数，
    // 若不单独判空会落到 InvalidLength 并给出"长度 0 不是 16 的倍数"这种错误信息。
    assert!(matches!(err, HlsCryptoError::Empty), "实际: {err}");
}
