.PHONY: fmt
fmt:
	cargo +nightly fmt

.PHONY: lint-toml
lint-toml: ensure-dprint
	dprint fmt

ensure-dprint:
	@if ! command -v dprint &> /dev/null; then \
		echo "dprint not found. Please install it by running the command `cargo install --locked dprint` or refer to the following link for more information: https://github.com/dprint/dprint" \
		exit 1; \
    fi

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

.PHONY: clippy-fix
clippy-fix:
	cargo +nightly clippy \
	--workspace \
	--lib \
	--examples \
	--tests \
	--benches \
	--all-features \
	--fix \
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
		echo "zepter not found. Please follow the instructions for installation: https://github.com/ggwpez/zepter?tab=readme-ov-file#install" \
		exit 1; \
    fi

.PHONY: lint
lint: fmt lint-toml clippy udeps codespell zepter

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

.PHONY: docker
docker:
	docker build -t scrolltech/rollup-node:latest . -f Dockerfile

.PHONY: docker-multiarch
docker-multiarch:
	docker buildx build --platform linux/amd64,linux/arm64 -t scrolltech/rollup-node:latest . -f Dockerfile

.PHONY: docker-setup-buildx
docker-setup-buildx:
	docker buildx create --name multiarch --driver docker-container --bootstrap --use || docker buildx use multiarch
