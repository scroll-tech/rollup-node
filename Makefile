.PHONY: fmt
fmt:
	cargo +nightly fmt

.PHONY: clippy
clippy:
	cargo +nightly clippy \
	--workspace \
	--lib \
	--examples \
	--tests \
	--benches \
	--all-features \
	-- -D warnings

.PHONY: udeps
udeps:
	cargo +nightly udeps --workspace --lib --examples --tests --benches --all-features --locked

.PHONY: codespell
codespell: ensure-codespell
	codespell --skip "*.json"

ensure-codespell:
	@if ! command -v codespell &> /dev/null; then \
		echo "codespell not found. Please install it by running the command `pip install codespell` or refer to the following link for more information: https://github.com/codespell-project/codespell" \
		exit 1; \
    fi

.PHONY: zepter
zepter: ensure-zepter
	zepter run check

ensure-zepter:
	@if ! command -v zepter &> /dev/null; then \
		echo "zepter not found. Please follow the instructions provider for installation: https://github.com/ggwpez/zepter?tab=readme-ov-file#install" \
		exit 1; \
    fi

.PHONY: lint
lint: fmt clippy udeps codespell zepter

.PHONY: test
test:
	cargo test \
	--workspace \
	--locked \
	--all-features \
	--no-fail-fast

.PHONY: docs
docs:
	cargo docs --document-private-items

.PHONY: pr
pr: lint test docs
