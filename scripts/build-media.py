"""Compile the local macOS PDFKit/Vision adapter into the requested output path."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target/debug/rust-word-media"
output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run(["xcrun", "swiftc", "-O", "-file-prefix-map", str(Path.home())+"=/build", "-framework", "PDFKit", "-framework", "Vision",
                "-framework", "AppKit", str(ROOT / "scripts/media-helper.swift"),
                "-o", str(output)], check=True)
print(output)
