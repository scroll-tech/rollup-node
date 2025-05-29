# Multi-Architecture Docker Image Build Guide

## Overview

This project now supports building multi-architecture Docker images for both `linux/amd64` and `linux/arm64` platforms.

## Prerequisites

1. Docker version >= 19.03
2. Docker Buildx (usually included in newer Docker versions)

## Quick Start

### 1. Setup Docker Buildx

Before first use, you need to set up the buildx builder:

```bash
make docker-setup-buildx
```

Or execute manually:

```bash
docker buildx create --name multiarch --driver docker-container --bootstrap --use
```

### 2. Build Multi-Architecture Images

#### Local Build (without pushing to registry)

```bash
make docker-multiarch
```

This will build images for both `linux/amd64` and `linux/arm64` architectures, but will only save them in the local build cache.


### 3. Single Architecture Build (Traditional Way)

If you only need to build an image for the current platform:

```bash
make docker
```

## Technical Details

### Dockerfile Modifications

1. **Multi-platform Base Images**: Uses `--platform=$BUILDPLATFORM` to ensure base images support multi-architecture
2. **Cross-compilation Support**: 
   - Added `gcc-aarch64-linux-gnu` cross-compiler for ARM64
   - Configured corresponding Rust targets and linkers
3. **Conditional Building**: Selects the correct compilation target based on `$TARGETPLATFORM` environment variable
4. **Smart Binary Copying**: Copies the correct binary file based on the target platform

### Supported Platforms

- `linux/amd64` (x86_64)
- `linux/arm64` (aarch64)

### Build Time Optimization

- Uses `cargo-chef` to cache dependency builds
- Leverages Docker multi-stage builds to reduce final image size
- BuildKit cache mechanism improves repeated build performance

## Troubleshooting

### Issue: buildx not available

**Solution**: Update Docker to the latest version, or manually install buildx:

```bash
# Check if buildx is available
docker buildx version

# If not available, you can install it with:
docker buildx install
```

### Issue: Cross-compilation failures

**Solution**: Ensure your system has sufficient memory and storage space, as cross-compilation typically requires more resources.

### Issue: ARM64 builds are slow on Apple Silicon Macs

**Solution**: This is normal behavior, as even on ARM64 hosts, Docker still needs to emulate the Linux ARM64 environment.

## Verifying Build Results

After the build is complete, you can check if the image contains multiple architectures:

```bash
docker buildx imagetools inspect scrolltech/rollup-node:latest
```

## Usage Recommendations

1. **Development Phase**: Use `make docker` for fast single-architecture builds
2. **Testing Phase**: Use `make docker-multiarch` for multi-architecture validation
3. **Release Phase**: GitHub Actions will automatically build and push multi-architecture images when you create a release or push a tag

## GitHub Actions Integration

The project includes a GitHub Actions workflow (`.github/workflows/release.yml`) that automatically builds and pushes multi-architecture images to Docker Hub when:

- A new tag is pushed
- A GitHub release is published

### Workflow Features

- **Multi-platform builds**: Automatically builds for `linux/amd64` and `linux/arm64`
- **Smart caching**: Uses GitHub Actions cache to speed up builds
- **Automatic tagging**: Tags images based on Git tags and releases
- **Secure authentication**: Uses Docker Hub credentials stored in GitHub Secrets

### Required Secrets

Make sure the following secrets are configured in your GitHub repository:

- `DOCKERHUB_USERNAME`: Your Docker Hub username
- `DOCKERHUB_TOKEN`: Your Docker Hub access token

## Performance Notes
