# Packaging channels

The tag build (`.github/workflows/release.yml`) publishes four archives plus, since the
packaging step was added:

- `checksums.txt` — sha256 of every archive.
- `nook.rb` — ready-to-use Homebrew formula, rendered with the release version and hashes.
- `nook.json` — ready-to-use Scoop manifest, likewise.

Templates live in this directory; CI renders them with `@…@` placeholder substitution
(`@VERSION@`, `@URL_BASE@`, `@SHA256_AARCH64@`, `@SHA256_X86_64@`, `@SHA256_WINDOWS@`).

## Homebrew (macOS)

One-time setup:

1. Create a GitHub repo named `homebrew-nook` (this name makes brew's short-form tap syntax work).
2. Add a `Formula/` directory.

Per release (after tagging, once the release is published):

1. Download `nook.rb` from the release assets.
2. `cp nook.rb <tap-clone>/Formula/nook.rb && git commit -m "nook <version>" && git push`.

Users install with:

```sh
brew install kasfulk/nook/nook
```

macOS binaries are not codesigned/notarised; the tarball install via brew does not trigger
Gatekeeper quarantine (no `com.apple.quarantine` xattr is set by curl/brew). A plain
browser download does — clear it with `xattr -d com.apple.quarantine nook`.

## Scoop (Windows)

One-time setup:

1. Create a GitHub repo named `scoop-nook` (a Scoop bucket).

Per release:

1. Download `nook.json` from the release assets into the bucket root.
2. Commit and push. Scoop's `checkver: github` + `autoupdate` (hashes read from
   `checksums.txt`) keep the bucket current after the initial commit.

Users install with:

```powershell
scoop bucket add nook https://github.com/kasfulk/scoop-nook
scoop install nook
```

## Linux

No package-manager channel yet. The x86_64 tarball links `libwayland-client` and
`libxkbcommon` (build-time; install them with the system package manager if missing).
Candidate upgrades later: AppImage (portable), `.deb` (Ubuntu CI), or a Homebrew-on-Linux
formula once deps are declared.

## Possible upgrade: automated tap/bucket push

The manual copy step can be automated by pushing `nook.rb`/`nook.json` from the release job
with a fine-grained PAT stored as a repository secret. Deliberately not wired up — add it
if manual copying becomes annoying.
