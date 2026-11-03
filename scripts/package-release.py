"""Package explicitly selected Windows release files; never includes local device data."""
import hashlib
import pathlib
import subprocess
import tomllib
import zipfile

root = pathlib.Path(__file__).resolve().parents[1]
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
name = f'serial-workbench-{version}-windows-x86_64'
out = root / 'artifacts' / 'releases'
out.mkdir(parents=True, exist_ok=True)
files = {
    'serial-workbench.exe': root / 'target/release/serial-workbench.exe',
    'serial-mcp-server.exe': root / 'target/release/serial-mcp-server.exe',
    'THIRD_PARTY_NOTICES.txt': root / 'web/THIRD_PARTY_NOTICES.txt',
}
for relative in ['README.md', 'README_ZH.md', 'LICENSE', 'CHANGELOG.md',
                 'start-workbench.cmd', 'scripts/start-workbench.ps1',
                 'docs/WORKBENCH_ZH.md', 'docs/LEGACY_CLI.md', 'docs/LEGACY_CLI_ZH.md',
                 'docs/releases/v' + version + '.md']:
    files[relative] = root / relative
for binary in ['serial-workbench.exe', 'serial-mcp-server.exe']:
    actual = subprocess.check_output([str(files[binary]), '--version'], text=True).strip()
    assert actual.endswith(' ' + version), actual
checksums = ''.join(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {relative}\n'
                    for relative, path in sorted(files.items()))
archive = out / (name + '.zip')
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as zip_file:
    for relative, path in files.items():
        zip_file.write(path, name + '/' + relative)
    zip_file.writestr(name + '/SHA256SUMS.txt', checksums)
checksum = out / (name + '.zip.sha256')
checksum.write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n', encoding='ascii')
print(archive)
print(checksum)
