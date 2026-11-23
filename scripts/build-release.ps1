# Builds a release .exe that's safe to share: no folder names from this PC end up inside it.
$root = Split-Path $PSScriptRoot -Parent
$env:RUSTFLAGS = "-C target-feature=+crt-static -C link-arg=/PDBALTPATH:%_PDB% --remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=$root=."
Push-Location $root
try { cargo build --release --locked; $code = $LASTEXITCODE } finally { Pop-Location; Remove-Item Env:RUSTFLAGS }
exit $code
