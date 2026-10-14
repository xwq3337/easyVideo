use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use thiserror::Error;

type Aes128CbcDecrypt = cbc::Decryptor<aes::Aes128>;

#[derive(Error, Debug)]
pub enum HlsCryptoError {
    #[error("ciphertext length {0} is not a multiple of 16")]
    InvalidLength(usize),
    #[error("decrypt failed: {0}")]
    Decrypt(String),
}

pub fn decrypt_aes128(
    key: &[u8; 16],
    iv: &[u8; 16],
    data: &[u8],
) -> Result<Vec<u8>, HlsCryptoError> {
    if data.len() % 16 != 0 || data.is_empty() {
        return Err(HlsCryptoError::InvalidLength(data.len()));
    }
    let mut buf = data.to_vec();
    let decryptor = Aes128CbcDecrypt::new(key.into(), iv.into());
    if let Ok(pt) = decryptor.decrypt_padded_mut::<Pkcs7>(&mut buf) {
        return Ok(pt.to_vec());
    }
    // decrypt_padded_mut 是原地解密：去填充失败时 buf 已被改写为明文，回退前必须还原密文。
    buf.copy_from_slice(data);
    // 非标准填充：按无填充解密，原样保留全部字节（TS 播放器可容忍尾部无填充）
    let decryptor = Aes128CbcDecrypt::new(key.into(), iv.into());
    let pt = decryptor
        .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
        .map_err(|e| HlsCryptoError::Decrypt(format!("{e:?}")))?;
    Ok(pt.to_vec())
}
