use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use thiserror::Error;

type Aes128CbcDecrypt = cbc::Decryptor<aes::Aes128>;

#[derive(Error, Debug)]
pub enum HlsCryptoError {
    #[error("ciphertext is empty")]
    Empty,
    #[error("ciphertext length {0} is not a multiple of 16")]
    InvalidLength(usize),
    #[error(
        "PKCS7 去填充失败：分片已损坏，或密钥/IV 不匹配。\
HLS AES-128 不带 MAC，PKCS7 校验是它唯一的完整性信号，因此这里必须报错"
    )]
    BadPadding,
}

pub fn decrypt_aes128(
    key: &[u8; 16],
    iv: &[u8; 16],
    data: &[u8],
) -> Result<Vec<u8>, HlsCryptoError> {
    if data.is_empty() {
        return Err(HlsCryptoError::Empty);
    }
    if data.len() % 16 != 0 {
        return Err(HlsCryptoError::InvalidLength(data.len()));
    }
    // 用原地解密 API 而非 `decrypt_padded_vec_mut`：后者需要 cipher 的 `alloc` feature，
    // 而当前解析到的 cipher v0.4.4 未启用（启用它等于改依赖）。
    //
    // **不做"去填充失败就按无填充返回"的宽容回退**。原因：
    // 1) 密钥/IV 正确时，解出的最后一个分组本来就是原始 PKCS7 填充，去填充必然成功。
    //    因此失败只可能意味着密钥/IV 错或数据损坏 —— 此时解出的**整段**都是乱码，
    //    不只是尾部，返回它没有任何好处。
    // 2) HLS AES-128 没有 MAC：PKCS7 校验是格式提供的**唯一**完整性信号。
    //    把它降级成"宽容通过"，等于把每一次本可检测的损坏变成静默的坏视频。
    let mut buf = data.to_vec();
    let decryptor = Aes128CbcDecrypt::new(key.into(), iv.into());
    match decryptor.decrypt_padded_mut::<Pkcs7>(&mut buf) {
        Ok(pt) => Ok(pt.to_vec()),
        Err(_) => Err(HlsCryptoError::BadPadding),
    }
}
