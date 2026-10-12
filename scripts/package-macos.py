#!/usr/bin/env python3
"""Create a standalone, locally signed macOS application after validation."""
from pathlib import Path
import plistlib
import os
import shutil
import subprocess
import sys
import tomllib

root=Path(__file__).resolve().parents[1]
profile='debug' if '--debug' in sys.argv else 'release'
binary=root/'target'/profile/'rust-word'
if not binary.exists():raise SystemExit(f'Build first: cargo build {"--release" if profile=="release" else ""}')
stage=root/'build'/'Rust Word.stage.app'
target=root/'build'/'Rust Word.app'
if stage.exists():shutil.rmtree(stage)
(stage/'Contents'/'MacOS').mkdir(parents=True)
shutil.copy2(binary,stage/'Contents'/'MacOS'/'rust-word')
subprocess.run([sys.executable,str(root/'scripts/build-media.py'),str(stage/'Contents'/'MacOS'/'rust-word-media')],check=True)
resources=stage/'Contents'/'Resources'
resources.mkdir(parents=True,exist_ok=True)
lo=Path(os.environ.get('RUST_WORD_LIBREOFFICE_APP','/Applications/LibreOffice.app'))
if not lo.is_dir():raise SystemExit('Set RUST_WORD_LIBREOFFICE_APP to a complete LibreOffice application bundle')
shutil.copytree(lo,resources/'LibreOfficeDev.app',symlinks=True)
rclone=Path(os.environ.get('RUST_WORD_RCLONE') or shutil.which('rclone') or '/nonexistent/rclone')
if not rclone.is_file():raise SystemExit('Install rclone or set RUST_WORD_RCLONE')
shutil.copy2(rclone,stage/'Contents'/'MacOS'/'rclone')
license_path=Path(os.environ.get('RUST_WORD_RCLONE_LICENSE',str(rclone.resolve().parent.parent/'COPYING')))
if not license_path.is_file():raise SystemExit('Set RUST_WORD_RCLONE_LICENSE to the rclone license file')
shutil.copy2(license_path,resources/'rclone-LICENSE.txt')
(resources/'THIRD_PARTY_NOTICES.md').write_text((root/'THIRD_PARTY_NOTICES.md').read_text(),encoding='utf-8')
shutil.copy2(root/'LICENSE', resources/'Rust-Word-LICENSE.txt')

version=tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
info={
 'CFBundleName':'Rust Word','CFBundleDisplayName':'Rust Word','CFBundleIdentifier':'io.github.haiy.rust-word',
 'CFBundleExecutable':'rust-word','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':'4', 'CFBundleDevelopmentRegion':'en','CFBundleLocalizations':['en','zh_CN'],
 'NSHighResolutionCapable':True,'LSMinimumSystemVersion':'12.0','NSPrincipalClass':'NSApplication',
 'CFBundleDocumentTypes':[{'CFBundleTypeName':'Word document','CFBundleTypeExtensions':['docx'],'CFBundleTypeRole':'Editor','LSHandlerRank':'Alternate'}]
}
(stage/'Contents'/'Info.plist').write_bytes(plistlib.dumps(info))
subprocess.run(['codesign','--force','--deep','--sign','-',str(stage)],check=True)
subprocess.run(['codesign','--verify','--deep','--strict',str(stage)],check=True)
if target.exists():
 previous=root/'build'/'Rust Word.previous.app'
 if previous.exists():shutil.rmtree(previous)
 target.rename(previous)
stage.rename(target)
print(target)
