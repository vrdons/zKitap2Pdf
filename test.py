import base64
from Crypto.Cipher import Blowfish
from Crypto.Util.Padding import unpad
import json

# -----------------------------
# KK sınıfı (AS3 KK karşılığı)
# -----------------------------
class KKDecryptor:
    def decrypt(self, data: str, key_str: str, apply_fd2: bool = True) -> str:
        if apply_fd2:
            data = self.fd2_transform(data, key_str)

        # Base64 decode
        file_bytes = base64.b64decode(data)

        # Blowfish-ECB decrypt
        key_bytes = key_str.encode('utf-8')
        cipher = Blowfish.new(key_bytes, Blowfish.MODE_ECB)
        decrypted_bytes = cipher.decrypt(file_bytes)

        # PKCS5 unpad
        try:
            decrypted_bytes = unpad(decrypted_bytes, Blowfish.block_size)
        except ValueError:
            pass

        return decrypted_bytes.decode('utf-8', errors='ignore')

    def fd2_transform(self, input_str: str, key: str) -> str:
        reversed_input = input_str[::-1]
        input_bytes = base64.b64decode(reversed_input)
        xor_bytes = self.apply_xor(input_bytes, key)
        return xor_bytes.decode('utf-8', errors='ignore')

    @staticmethod
    def apply_xor(input_bytes: bytes, key: str) -> bytes:
        key_bytes = key.encode('utf-8')
        key_len = len(key_bytes)
        out_bytes = bytearray()
        for i, b in enumerate(input_bytes):
            out_bytes.append(b ^ key_bytes[i % key_len])
        return bytes(out_bytes)

# -----------------------------
# KrySWFCrypto sınıfı (AS3 karşılığı)
# -----------------------------
class SWFCrypto:
    def decrypt_bytes(self, data: bytearray, code_map: dict) -> bytearray:
        self.separate_bytes(data, 10000, 11000, code_map['f1'], code_map['f2'])
        self.separate_bytes(data, 5000, 5500, code_map['f3'], code_map['f1'])
        self.separate_bytes(data, 850, 1500, code_map['f2'], code_map['f3'])
        self.separate_bytes(data, 0, 300, code_map['f1'], code_map['f2'])

        # Son eklemeler (mod 256)
        data[code_map['f3']] = (data[code_map['f3']] - code_map['f3']) % 256
        data[code_map['f2']] = (data[code_map['f2']] - code_map['f2']) % 256
        data[code_map['f1']] = (data[code_map['f1']] - code_map['f1']) % 256
        data[2] = (data[2] - code_map['f3']) % 256
        data[1] = (data[1] - code_map['f2']) % 256
        data[0] = (data[0] - code_map['f1']) % 256

        return data

    @staticmethod
    def separate_bytes(data: bytearray, start: int, end: int, n1: int, n2: int):
        temp = [ (data[i] - n2) % 256 for i in range(start, end + n1 * 3) ]
        temp.reverse()
        for k, i in enumerate(range(start, end + n1 * 3)):
            data[i] = (temp[k] - n2) % 256

# -----------------------------
# CK fonksiyonu (lisans kontrol mantığı)
# -----------------------------
def check_key(my_key: str, kk_object: dict, pkxk_name: str) -> bool:
    chars = list("ABCDEFGHJKLMNPRSTVYZ123456789")
    key_chars = list(my_key.upper())

    if len(my_key) < 8:
        return False

    # AS3 kodundaki totKey hesaplaması
    tot_key = sum(ord(my_key[i]) for i in [0,3,4,5,7])

    idx1 = 0 % (int(kk_object['f1']) - len(pkxk_name))
    idx2 = 0 % (int(kk_object['f2']) - len(pkxk_name))
    idx3 = 0 % (int(kk_object['f3']) - len(pkxk_name))
    print(key_chars,int(kk_object['f1']),int(kk_object['f2']),int(kk_object['f3']));
    if chars[idx1] == key_chars[1] and chars[idx2] == key_chars[2] and chars[idx3] == key_chars[6]:
        return True

    return False

# -----------------------------
# Örnek kullanım
# -----------------------------
if __name__ == "__main__":
    key = "pub1isher1l0O"
    kk = KKDecryptor()

    # Şifreli veriyi oku
    with open("enc.txt", "r") as f:
        encrypted_data = f.read().strip()

    # fd1 ile çöz
    decrypted_str = kk.decrypt(encrypted_data, key, apply_fd2=True)

    # Base64 decode
    byte_data = bytearray(base64.b64decode(decrypted_str))

    # Fernus kodunu çöz ve f1,f2,f3 hesapla
    fernus_code = kk.decrypt("RxQFOUiQdw1D2ACf8dyW8ERWEIEcEMiJ", key)
    parts = fernus_code.split("x")
    kk_object = {
        'f1': int(parts[0]) + len("fernus"),
        'f2': int(parts[1]) + len("fernus"),
        'f3': int(parts[2]) + len("fernus")
    }

    # KrySWFCrypto ile decryption
    crypto = SWFCrypto()
    decrypted_bytes = crypto.decrypt_bytes(byte_data, kk_object)

    # p.dll olarak yaz
    with open("byte.dll", "wb") as f:
        f.write(decrypted_bytes)

    print("/tmp/p.dll (CWS code) yazıldı.")

    # Byte.txt içeriğini JSON çözme
    with open("byte.txt", "r") as f:
        encrypted_json_str = f.read().strip()

    json_text = kk.decrypt(encrypted_json_str, key)
    j = json.loads(json_text)

    fernus_code_decoded = kk.decrypt(j["fernusCode"], key)
    print(fernus_code_decoded)
    parts2 = fernus_code_decoded.split("x")
    kk_object2 = {
        'f1': int(parts2[0]) + len(j["pkxkname"]),
        'f2': int(parts2[1]) + len(j["pkxkname"]),
        'f3': int(parts2[2]) + len(j["pkxkname"])
    }
    print(j["fernusCode"], kk_object2, j["pkxkname"])
    print("CK kontrol sonucu:", check_key(j["fernusCode"], kk_object2, j["pkxkname"]))
