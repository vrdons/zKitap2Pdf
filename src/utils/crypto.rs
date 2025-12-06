use base64::{engine::general_purpose, Engine as _};
use blowfish::Blowfish;
use cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use byteorder::BigEndian;

pub fn decrypt_publisher(input: Vec<u8>, key: &str) -> () {
 let key_bytes = key.as_bytes();
    let fd2_bytes = fd2(&input, key_bytes);
    let decrypted = blowfish_ecb_decrypt(&fd2_bytes, key_bytes);

    println!("{}", String::from_utf8(decrypted).expect("Invalid UTF-8 after decrypt"));
}

fn blowfish_ecb_decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    let cipher: Blowfish<BigEndian> = Blowfish::new_from_slice(key).expect("Invalid key");

    let mut decrypted = vec![];
    for block in data.chunks(8) {
        let mut block_arr = [0u8; 8];
        block_arr[..block.len()].copy_from_slice(block);

        let mut block_ga = GenericArray::from_mut_slice(&mut block_arr);
        cipher.decrypt_block(&mut block_ga);

        decrypted.extend_from_slice(&block_arr);
    }

    pkcs5_unpad(&decrypted)
}

fn pkcs5_unpad(data: &[u8]) -> Vec<u8> {
    let pad_len = *data.last().expect("Empty data") as usize;
    println!("Pad length: {}", pad_len);
    let len = data.len() - pad_len;
    data[..len].to_vec()
}
fn fd2(input: &[u8], key: &[u8]) -> Vec<u8> {
    let s = String::from_utf8(input.to_vec()).expect("Invalid UTF-8");
    let rev_input: String = s.chars().rev().collect();
    let decoded = general_purpose::STANDARD.decode(rev_input).expect("Base64 decode failed");
    apply_xor(&decoded, key)
}
fn apply_xor(input: &[u8], key: &[u8]) -> Vec<u8> {
    input.iter()
         .enumerate()
         .map(|(i, &b)| b ^ key[i % key.len()])
         .collect()
}