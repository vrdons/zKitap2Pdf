import base64
from Crypto.Cipher import Blowfish
from Crypto.Util.Padding import unpad
import json
# -----------------------------
# KK sınıfı
# -----------------------------
class KK:
    def fd1(self, data: str, key_str: str, _s: bool = True) -> str:
        if _s:
            data = self.fd2(data, key_str)
        
        # Base64 decode
        file_bytes = base64.b64decode(data)
        
        # Blowfish-ECB decrypt
        key_bytes = key_str.encode('utf-8')
        cipher = Blowfish.new(key_bytes, Blowfish.MODE_ECB)
        decrypted = cipher.decrypt(file_bytes)
        
        # PKCS5 unpad
        try:
            decrypted = unpad(decrypted, Blowfish.block_size)
        except ValueError:
            pass
        
        return decrypted.decode('utf-8', errors='ignore')

    def apply_xor(self, input_bytes: bytes, key: str) -> bytes:
        key_bytes = key.encode('utf-8')
        out_bytes = bytearray()
        key_len = len(key_bytes)
        for i, b in enumerate(input_bytes):
            out_bytes.append(b ^ key_bytes[i % key_len])
        return bytes(out_bytes)

    def fd2(self, input_str: str, key: str) -> str:
        reversed_input = input_str[::-1]
        input_bytes = base64.b64decode(reversed_input)
        out_bytes = self.apply_xor(input_bytes, key)
        return out_bytes.decode('utf-8', errors='ignore')

# -----------------------------
# KrySWFCrypto sınıfı
# -----------------------------
class KrySWFCrypto:
    def decrypte(self, _bytes: bytearray, _code: dict) -> bytearray:
        self._bytes = _bytes
        self._code = _code

        self.separate_bytes(self._bytes, 10000, 11000, _code['f1'], _code['f2'])
        self.separate_bytes(self._bytes, 5000, 5500, _code['f3'], _code['f1'])
        self.separate_bytes(self._bytes, 850, 1500, _code['f2'], _code['f3'])
        self.separate_bytes(self._bytes, 0, 300, _code['f1'], _code['f2'])

        # Son eklemeler (mod 256 ile wrap)
        self._bytes[_code['f3']] = (self._bytes[_code['f3']] - _code['f3']) % 256
        self._bytes[_code['f2']] = (self._bytes[_code['f2']] - _code['f2']) % 256
        self._bytes[_code['f1']] = (self._bytes[_code['f1']] - _code['f1']) % 256
        self._bytes[2] = (self._bytes[2] - _code['f3']) % 256
        self._bytes[1] = (self._bytes[1] - _code['f2']) % 256
        self._bytes[0] = (self._bytes[0] - _code['f1']) % 256

        return self._bytes

    def separate_bytes(self, _b: bytearray, _sIndex: int, _eIndex: int, _n1: int, _n2: int):
        temp_array = [(_b[i] - _n2) % 256 for i in range(_sIndex, _eIndex + _n1 * 3)]
        temp_array.reverse()
        for k, i in enumerate(range(_sIndex, _eIndex + _n1 * 3)):
            _b[i] = (temp_array[k] - _n2) % 256

# -----------------------------
# Örnek kullanım
# -----------------------------
if __name__ == "__main__":
    kk = KK()
    key = "pub1isher1l0O"

    # Şifreli veriyi oku
    with open("enc.txt", "r") as f:
        encrypted_data = f.read().strip()

    # fd1 ile çöz
    _s = kk.fd1(encrypted_data, key, _s=True)

    # Base64 decode
    _d = bytearray(base64.b64decode(_s))

    # _w çöz ve f1,f2,f3
    _w = kk.fd1("RxQFOUiQdw1D2ACf8dyW8ERWEIEcEMiJ", key)
    parts = _w.split("x")
    _f = {
        'f1': int(parts[0]) + len("fernus"),
        'f2': int(parts[1]) + len("fernus"),
        'f3': int(parts[2]) + len("fernus")
    }

    # KrySWFCrypto ile decrypte
    crypto = KrySWFCrypto()
    _b = crypto.decrypte(_d, _f)

    # p.dll dosyasına yaz
    #with open("/tmp/p.dll", "wb") as f:
    #    f.write(_b)

    print("/tmp/p.dll (CWS code) yazıldı. Artık içeriği doğru çözülebilir.")
    with open("byte.txt", "r") as f:
        encrypted_str = f.read().strip()
    json_text = kk.fd1(encrypted_str, "pub1isher1l0O")
    j = json.loads(json_text)
    print(j)