#!/usr/bin/env python3
"""v3 Fernus Z-Kitap - Full automated decrypt pipeline
Extracts key + IV from kernel_blob.bin via envied XOR, decrypts everything.
Usage: python3 v3_pipeline.py [exe_or_extracted_dir]"""
import base64, json, os, re, struct, sys, subprocess
from pathlib import Path
from Crypto.Cipher import AES
from Crypto.Util.Padding import unpad


def extract_iv_from_kernel(kernel_path: str) -> bytes:
    """Extract IV from kernel_blob.bin: 'static final iv = IV.fromUtf8('...')"""
    with open(kernel_path, 'rb') as f:
        kb = f.read()
    patterns = [
        b"static final iv = IV.fromUtf8('",
        b"static const iv = IV.fromUtf8('",
    ]
    for pat in patterns:
        idx = kb.find(pat)
        if idx > 0:
            start = idx + len(pat)
            end = kb.find(b"');", start)
            if end > 0:
                return kb[start:end]
    # Fallback: search for any quoted string near 'iv ='
    for m in re.finditer(b"iv\\s*=\\s*IV\\.fromUtf8\\('([^']+)'", kb):
        return m.group(1)
    raise ValueError("Could not find IV in kernel_blob.bin")


def extract_keystr_from_kernel(kernel_path: str) -> bytes:
    """Extract keystr by XORing envied key/data arrays from kernel_blob.bin."""
    with open(kernel_path, 'rb') as f:
        kb = f.read()

    def parse_envied_array(suffix: str) -> list[int]:
        """Parse _enviedkey_keyStr or _envieddata_keyStr arrays."""
        needle = f'_envied{suffix}_keyStr = <int>['.encode()
        idx = kb.find(needle)
        if idx == -1:
            raise ValueError(f"Could not find {needle!r}")
        start = idx + len(needle)
        end = kb.find(b'];', start)
        if end == -1:
            raise ValueError(f"Could not find end of {suffix} array")
        text = kb[start:end].decode('ascii', errors='replace')
        return [int(x.strip().strip(',')) for x in text.split() if x.strip().strip(',').lstrip('-').isdigit()]

    key_ints = parse_envied_array('key')
    data_ints = parse_envied_array('data')

    if len(key_ints) != len(data_ints):
        raise ValueError(f"Array length mismatch: key={len(key_ints)}, data={len(data_ints)}")

    # XOR each int (envied stores one character per int in low byte)
    chars = []
    for k, d in zip(key_ints, data_ints):
        c = (k ^ d) & 0xFF
        if c != 0:
            chars.append(chr(c))
    return ''.join(chars).encode()


def create_key(fernus_code: str, keystr: bytes) -> bytes:
    """Dart: createKey(String key) -> key + keystr.substring(0, 32 - key.length)"""
    key = fernus_code.encode()
    if len(key) < 32:
        key = key + keystr[:32 - len(key)]
    return key[:32]


def decrypt_string(encrypted_str: str, key: bytes, iv: bytes) -> bytes:
    """Dart: decryptString: reverse -> base64decode -> AES-CBC -> PKCS7 unpad"""
    reversed_str = encrypted_str[::-1]
    ciphertext = base64.b64decode(reversed_str + '==', validate=False)
    cipher = AES.new(key, AES.MODE_CBC, iv=iv)
    return unpad(cipher.decrypt(ciphertext), 16)


def decrypt_bytes(data: bytes, key: bytes, iv: bytes) -> bytes:
    """Dart: decryptByte: AES-CBC -> PKCS7 unpad (direct binary, no base64)"""
    cipher = AES.new(key, AES.MODE_CBC, iv=iv)
    return unpad(cipher.decrypt(data), 16)


def unpack_enigma(exe_path: str) -> Path:
    """Unpack Enigma Virtual Box EXE using evbunpack, return extracted dir."""
    exe_path = Path(exe_path).resolve()
    if not exe_path.exists():
        raise FileNotFoundError(f"EXE not found: {exe_path}")

    # Use exe name for output dir
    out_dir = Path(f'/tmp/v3_{exe_path.stem}')
    if out_dir.exists() and (out_dir / 'publisher' / 'book').is_dir():
        print(f"  Using cached extraction: {out_dir}")
        return out_dir

    print(f"  Unpacking with evbunpack...")
    result = subprocess.run(
        ['evbunpack', str(exe_path), str(out_dir)],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(f"evbunpack failed: {result.stderr}")

    if not out_dir.exists():
        raise FileNotFoundError(f"evbunpack output not found: {out_dir}")
    return out_dir


def discover_extracted_dir(path: str) -> Path:
    """Find the directory containing publisher/book/ and kernel_blob.bin.
    If path is an EXE, unpack it first. If it's a directory, use directly."""
    path = Path(path)

    if path.suffix.lower() == '.exe':
        # It's an EXE - unpack via evbunpack
        return unpack_enigma(str(path))

    # It's a directory - just verify
    if not path.exists():
        raise FileNotFoundError(f"Path not found: {path}")

    # Check for publisher/book/ (single-book layout)
    if (path / 'publisher' / 'book').is_dir():
        return path

    # Check for data/flutter_assets/kernel_blob.bin (evbunpack layout)
    if (path / 'data' / 'flutter_assets' / 'kernel_blob.bin').exists():
        return path

    raise FileNotFoundError(
        f"Could not find publisher/book/ or kernel_blob.bin in {path}"
    )


def main():
    print("=" * 60)
    print("v3 Fernus Z-Kitap - AUTO DECRYPT PIPELINE")
    print("=" * 60)

    # --- Input ---
    if len(sys.argv) > 1:
        input_path = sys.argv[1]
    else:
        input_path = os.environ.get('V3_DIR', '/tmp/v3_output')
    print(f"Input: {input_path}")

    # --- Unpack/Discover ---
    extracted_dir = discover_extracted_dir(input_path)
    print(f"VFS dir: {extracted_dir}")

    kernel_path = extracted_dir / 'data' / 'flutter_assets' / 'kernel_blob.bin'
    if not kernel_path.exists():
        raise FileNotFoundError(f"kernel_blob.bin not found at {kernel_path}")

    publisher_json = extracted_dir / 'publisher' / 'publisher.json'
    book_dir = extracted_dir / 'publisher' / 'book'
    book_json = book_dir / 'book.json'

    # --- Step 1: Extract keystr & IV from kernel ---
    print("\n[1/6] Extracting keystr & IV from kernel_blob.bin...")
    keystr = extract_keystr_from_kernel(str(kernel_path))
    print(f"  keystr: {keystr.decode()} (len={len(keystr)})")

    IV = extract_iv_from_kernel(str(kernel_path))
    print(f"  IV:     {IV.decode()} (len={len(IV)})")

    # --- Step 2: Decrypt publisher.json ---
    print(f"\n[2/6] Decrypting publisher.json...")
    with open(publisher_json, 'r') as f:
        pub_enc = f.read().strip()

    def _decrypt_string(data: str, key: bytes) -> bytes:
        return decrypt_string(data, key, IV)

    pub = json.loads(_decrypt_string(pub_enc, keystr))
    print(f"  pkxkname: {pub['pkxkname']}")
    print(f"  publisher: {pub.get('publisher', 'N/A')}")

    fernus_code = _decrypt_string(pub['fernusCode'], keystr).decode()
    print(f"  fernusCode (decrypted): {fernus_code}")

    # --- Step 3: Create book key ---
    print(f"\n[3/6] Creating book key...")
    book_key = create_key(fernus_code, keystr)
    print(f"  book_key: {book_key.decode()} (len={len(book_key)})")

    # --- Step 4: Decrypt book.json ---
    print(f"\n[4/6] Decrypting book.json...")
    with open(book_json, 'r') as f:
        book_enc = f.read().strip()
    book_data = json.loads(_decrypt_string(book_enc, book_key))
    print(f"  bookName: {book_data.get('bookName', 'N/A')}")
    print(f"  totalPage: {book_data.get('totalPage', 'N/A')}")

    out_dir = Path(f'/tmp/v3_{pub["pkxkname"]}_decrypted')
    out_dir.mkdir(exist_ok=True)
    with open(out_dir / 'book.json', 'w', encoding='utf-8') as f:
        json.dump(book_data, f, indent=2, ensure_ascii=False)

    # --- Step 5: Decrypt all webp files ---
    print(f"\n[5/6] Decrypting webp files from {book_dir}...")
    stats = {'pages': 0, 'layers': 0, 'thumbs': 0, 'failed': 0}

    def _decrypt_bytes(data: bytes, key: bytes) -> bytes:
        return decrypt_bytes(data, key, IV)

    for fn in sorted(os.listdir(book_dir)):
        if not fn.endswith('.webp'):
            continue
        path = book_dir / fn
        with open(path, 'rb') as f:
            enc = f.read()

        # Thumbnails (t-*.webp) are NOT encrypted — they have RIFF header
        if enc[:4] == b'RIFF':
            dec = enc
            cat = 'thumbs'
        else:
            try:
                dec = _decrypt_bytes(enc, book_key)
                if fn.startswith('p-l-'):
                    cat = 'layers'
                else:
                    cat = 'pages'
            except Exception as e:
                print(f"  FAILED: {fn}: {e}")
                stats['failed'] += 1
                continue

        with open(out_dir / fn, 'wb') as f:
            f.write(dec)
        stats[cat] += 1

    print(f"  Pages:  {stats['pages']}")
    print(f"  Layers: {stats['layers']}")
    print(f"  Thumbs: {stats['thumbs']} (passthrough)")
    if stats['failed']:
        print(f"  FAILED: {stats['failed']}")
    print(f"\nOutput: {out_dir}")

    # --- Step 6: Convert webp → PDF ---
    print(f"\n[6/6] Converting webp → PDF...")
    pdf_path = out_dir / f'{pub["pkxkname"]}_{book_data.get("bookName", "book")}.pdf'
    from PIL import Image
    page_nums = sorted(
        {int(fn.stem.split('-')[1]) for fn in out_dir.glob('p-*.webp') if not fn.name.startswith('p-l-')},
        key=int
    )
    images = []
    for pn in page_nums:
        img_path = out_dir / f'p-{pn}.webp'
        if img_path.exists():
            img = Image.open(img_path).convert('RGB')
            # Merge layer if exists
            layer_path = out_dir / f'p-l-{pn}.webp'
            if layer_path.exists():
                layer = Image.open(layer_path).convert('RGBA')
                img.paste(layer, (0, 0), layer)
            images.append(img)
    if images:
        images[0].save(pdf_path, save_all=True, append_images=images[1:])
        print(f"  PDF: {pdf_path} ({len(images)} pages)")
    else:
        print(f"  No pages found!")

    # --- Summary ---
    print("\n" + "=" * 60)
    print("DECRYPT COMPLETE ✅")
    print("=" * 60)
    print(f"""
Key chain:
  envied XOR → keystr = "{keystr.decode()}"
  kernel IV  → IV     = "{IV.decode()}"
  decrypt_string(publisher.fernusCode, keystr) → "{fernus_code}"
  createKey(fernusCode, keystr) → "{book_key.decode()}"

Output:
  JSON:   {out_dir}
  PDF:    {pdf_path}

For Rust implementation:
  - AES-256-CBC (crate: aes + cbc)
  - PKCS7 unpadding
  - IV: from kernel "static final iv"
  - keystr: from kernel envied XOR
  - decrypt_string: reverse → base64 decode → AES-CBC decrypt
  - decrypt_bytes:  AES-CBC decrypt (direct binary)
""")


if __name__ == '__main__':
    main()
