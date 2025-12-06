import subprocess
import os
import shutil

TARGET_DIR = "wine/drive_c/users/kullanc/AppData/Roaming/isler-vaf-2022-11-sinif-matematik-vaf-6/Local Store"
TARGET_DIR = os.path.abspath(TARGET_DIR)

print("İzlenen klasör:", TARGET_DIR)

OUT_DIR = "/tmp/sysdll"
os.makedirs(OUT_DIR, exist_ok=True)

# inotifywait komutu
cmd = [
    "inotifywait",
    "-m",
    "-r",
    TARGET_DIR,
    "--format", "%w%f %e",
    "-e", "create",
    "-e", "modify",
    "-e", "delete",
    "--exclude", ".*\\.tmp"
]

p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

print("✔ inotifywait başlatıldı, DLL bekleniyor...")

for line in p.stdout:
    line = line.strip()
    if line.endswith(".dll CREATE") or line.endswith(".dll MODIFY"):
        filepath, event = line.rsplit(" ", 1)
        filename = os.path.basename(filepath)
        dest = os.path.join(OUT_DIR, filename)

        try:
            shutil.copy2(filepath, dest)
            print(f"✔ DLL yakalandı: {filename} → {dest}")
        except FileNotFoundError:
            pass
