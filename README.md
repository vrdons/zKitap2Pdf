# zKitap2Pdf

zKitap2Pdf converts Fernus Z-Kitap EXEs into PDF pages using Ruffle. The current pipeline focuses on the runtime DLL assets released alongside the projector and keeps the implementation split into clear modules for crypto, rendering, and utilities.

[![Rust CI](https://img.shields.io/github/actions/workflow/status/vrdons/zKitap2Pdf/ci.yml?style=for-the-badge&label=Rust%20CI)](https://github.com/vrdons/zKitap2Pdf/actions/workflows/ci.yml)

## Installation

### Note
If you are using Linux, you may need to install Wine for launching the projector EXE.

### Prebuilt Binaries
You can download prebuilt binaries for **Linux** and **Windows** from the [Releases page](https://github.com/vrdons/zKitap2Pdf/releases).

| System / Distribution | File Extension | Description |
|:----------------------|:---------------|:------------|
| **Generic Linux** | `.tar.gz` | The most universal build. Extract and run the binary. |
| **Debian / Ubuntu** | `.deb` | Install using `dpkg`. |
| **Fedora / CentOS / openSUSE** | `.rpm` | For all RPM-based systems. |
| **Windows** | `.exe` or `.zip` | The standalone `.exe` is ready to run. The `.zip` contains the executable. |

### From Source
Requires **Git**, **Rust**, and **Cargo**:

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

## Contributing
Contributions are welcome. Issues and PRs are appreciated.

<a href="https://github.com/vrdons/zKitap2Pdf/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=vrdons/zKitap2Pdf" alt="zKitap2Pdf contributors" />
</a>

---

Created with 🩵 by [ErenayDev](https://erenaydev.com.tr) and [vrdons](https://github.com/vrdons)
