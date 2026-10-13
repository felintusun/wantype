# Common dev tasks. `just` lists them.

# Repo-local tools (Trunk, geckodriver) come first on PATH; those installed
# here are pinned, like the workflow's.
export PATH := (justfile_directory() / ".tools/bin") + ":" + env("PATH")

# List the tasks
default:
    @just --list --unsorted

# Build the sentence data from Tatoeba's exports unless it is already there
data:
    @cargo xtask data

# Dev server with live reload on http://127.0.0.1:PORT
[working-directory: 'app']
serve port="8080": data
    trunk serve --port {{port}}

# Release build of the site into target/site (apart from the dev server's
# app/dist), with relative paths so that it runs at any path (a GitHub Pages
# project page is served under /<repo>/)
[working-directory: 'app']
build: data
    trunk build --release --public-url ./ --dist ../target/site

# Unit tests, proptest invariants
test:
    cargo test --workspace

# Formatting and clippy, natively and for the wasm target; the workflows
# with zizmor
lint:
    @command -v zizmor >/dev/null || cargo install --locked --root .tools zizmor@1.30.1
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p wantype --target wasm32-unknown-unknown -- -D warnings
    zizmor --offline --persona=pedantic .github/workflows

# Format the code
fmt:
    cargo fmt --all

# Known vulnerabilities (RustSec) and unused dependencies
audit:
    @command -v cargo-audit >/dev/null || cargo install --locked --root .tools cargo-audit@0.22.2
    @command -v cargo-shear >/dev/null || cargo install --locked --root .tools cargo-shear@1.14.0
    cargo audit
    cargo shear

# Everything CI should run
check: lint audit test test-data

# The built data is consistent and every sentence typeable
test-data: data
    cargo test -p wantype-core --test data -- --ignored

# Release build played in headless Firefox; screenshots in target/smoke
smoke: build
    @command -v geckodriver >/dev/null || cargo install --locked --root .tools geckodriver@0.37.1
    cargo test -p e2e --test smoke -- --ignored

# The screens as markup for the design canvas, in target/screens
screens: build
    @command -v geckodriver >/dev/null || cargo install --locked --root .tools geckodriver@0.37.1
    cargo test -p e2e --test screens -- --ignored

# Fetch Tatoeba's newer exports, if any, and rebuild the sentence data
update-data:
    cargo xtask data --update

# Render the PNG icons from app/public/favicon.svg (a test checks they match)
icons:
    cargo xtask icons
