# zKitap2Pdf

zKitap2Pdf is a tool for converting Fernus Z-kitap e-books from exe to pdf format using ruffle

[![Rust CI](https://img.shields.io/github/actions/workflow/status/vrdons/zKitap2Pdf/ci.yml?style=for-the-badge&label=Rust%20CI)](https://github.com/vrdons/zKitap2Pdf/actions/workflows/ci.yml)

## Installation

### Note
If you are using Linux, you may need to install additional dependencies such as `wine`

### Prebuilt Binaries
You can download prebuilt binaries for **Linux** and **Windows** from the [Releases page](https://github.com/vrdons/zKitap2Pdf/releases).

| System / Distribution | File Extension | Description |
|:----------------------|:---------------|:------------|
| **Generic Linux** | `.tar.gz`      | The most universal build. Extract and run the binary. |
| **Debian / Ubuntu** | `.deb`         | Install using `dpkg`. |
| **Fedora / CentOS / openSUSE** | `.rpm`  | For all RPM-based systems. |
| **Windows** | `.exe` or `.zip` | The standalone **`.exe`** is ready to run. The **`.zip`** contains the executable. |

### From Source
Requires **Git**, **Rust**, **Cargo**:

```bash
git clone https://github.com/vrdons/zKitap2Pdf.git
cd zKitap2Pdf
cargo install --path .
```

After installation, you can run `zKitap2Pdf` in your terminal.

## CLI Arguments
Just a placeholder now. Coming soon..

## Cross-platform
We tested in Linux and Windows. It works fine. But we are not rich for buying a MacBook. See [#7](/../../issues/7)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=vrdons/zKitap2Pdf&type=date&legend=top-left)](https://www.star-history.com/#vrdons/zKitap2Pdf&type=date&legend=top-left)

## Contributing
Contributions are welcome! Issues, PR's etc.

<a href="https://github.com/vrdons/zKitap2Pdf/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=vrdons/zKitap2Pdf" alt="zKitap2Pdf contributors" />
</a>

---

Created with 🩵 by [ErenayDev](https://erenaydev.com.tr) and [vrdons](https://github.com/vrdons)
