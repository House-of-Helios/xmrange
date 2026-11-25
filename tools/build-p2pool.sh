#!/bin/bash
# Reproducible p2pool build: pinned tag, static deps (curl, libuv, libzmq), mirroring p2pool's own
# arm64 release recipe (.github/workflows/c-cpp.yml). Off: UPnP (XMRange never forwards ports),
# gRPC/Tari and the author-donation merge mining (both would open outbound connections beyond
# p2pool's peers). The binary links only system libraries, so it runs on any Apple Silicon Mac.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/vendor/p2pool"
TAG="v4.18.1"
PIN="abc833598f80d7a0345b6020ced90f19957d7a7c"
JOBS="$(sysctl -n hw.ncpu)"
# -ffile-prefix-map: no repo or home paths (the username) baked into the shipped binary.
TARGET="-target arm64-apple-macos11.3 -ffile-prefix-map=$SRC=p2pool -ffile-prefix-map=$HOME=~"

if [ ! -d "$SRC/.git" ]; then
  rm -rf "$SRC"   # pruned by tools/prune-vendor.sh: only finished binaries were left
  git clone --depth 1 --branch "$TAG" --recursive --shallow-submodules https://github.com/SChernykh/p2pool.git "$SRC"
fi
[ "$(git -C "$SRC" rev-parse HEAD)" = "$PIN" ] || { echo "p2pool is not at $TAG ($PIN)"; exit 1; }

# libcurl (HTTP only, no TLS: it only talks to monerod on 127.0.0.1)
(cd "$SRC/external/src/curl" && [ -f lib/libcurl.a ] || {
  cmake . -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-Os $TARGET" -DCURL_DROP_UNUSED=ON -DCURL_DISABLE_TYPECHECK=ON \
    -DCURL_DISABLE_OPENSSL_AUTO_LOAD_CONFIG=ON -DCURL_DISABLE_SHA512_256=ON -DPICKY_COMPILER=OFF -DBUILD_CURL_EXE=OFF \
    -DBUILD_SHARED_LIBS=OFF -DCURL_DISABLE_INSTALL=ON -DCURL_ENABLE_EXPORT_TARGET=OFF -DCURL_DISABLE_HEADERS_API=ON \
    -DCURL_DISABLE_BINDLOCAL=ON -DBUILD_LIBCURL_DOCS=OFF -DBUILD_MISC_DOCS=OFF -DENABLE_CURL_MANUAL=OFF -DCURL_ZLIB=OFF \
    -DCURL_BROTLI=OFF -DCURL_ZSTD=OFF -DCURL_DISABLE_ALTSVC=ON -DCURL_DISABLE_COOKIES=ON -DCURL_DISABLE_DOH=ON \
    -DCURL_DISABLE_GETOPTIONS=ON -DCURL_DISABLE_HSTS=ON -DCURL_DISABLE_LIBCURL_OPTION=ON -DCURL_DISABLE_MIME=ON \
    -DCURL_DISABLE_NETRC=ON -DCURL_DISABLE_NTLM=ON -DCURL_DISABLE_PARSEDATE=ON -DCURL_DISABLE_PROGRESS_METER=ON \
    -DCURL_DISABLE_SHUFFLE_DNS=ON -DCURL_DISABLE_VERBOSE_STRINGS=ON -DHTTP_ONLY=ON -DCURL_ENABLE_SSL=OFF -DUSE_LIBIDN2=OFF \
    -DCURL_USE_LIBPSL=OFF -DCURL_USE_LIBSSH2=OFF -DENABLE_UNIX_SOCKETS=OFF -DBUILD_TESTING=OFF -DUSE_NGHTTP2=OFF \
    -DBUILD_EXAMPLES=OFF -DP2POOL_BORINGSSL=ON -DCURL_DISABLE_SRP=ON -DCURL_DISABLE_AWS=ON -DCURL_DISABLE_BASIC_AUTH=ON \
    -DCURL_DISABLE_BEARER_AUTH=ON -DCURL_DISABLE_KERBEROS_AUTH=ON -DCURL_DISABLE_NEGOTIATE_AUTH=ON \
    -DOPENSSL_INCLUDE_DIR=../grpc/third_party/boringssl-with-bazel/include >/dev/null
  make -j"$JOBS" >/dev/null; })

(mkdir -p "$SRC/external/src/libuv/build" && cd "$SRC/external/src/libuv/build" && [ -f libuv.a ] || {
  cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-Os $TARGET" -DBUILD_TESTING=OFF -DLIBUV_BUILD_SHARED=OFF -DENABLE_CLANG_TIDY=OFF >/dev/null
  make -j"$JOBS" >/dev/null; })

(mkdir -p "$SRC/external/src/libzmq/build" && cd "$SRC/external/src/libzmq/build" && [ -f lib/libzmq.a ] || {
  cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-Os $TARGET" -DCMAKE_CXX_FLAGS="-Os $TARGET" -DWITH_LIBSODIUM=OFF \
    -DWITH_LIBBSD=OFF -DBUILD_TESTS=OFF -DWITH_DOCS=OFF -DENABLE_DRAFTS=OFF -DBUILD_SHARED=OFF >/dev/null
  make -j"$JOBS" >/dev/null; })

mkdir -p "$SRC/build"
# -Wno-unused-const-variable: with Tari off, its chain id constant is unused and -Werror trips.
cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$TARGET" -DCMAKE_CXX_FLAGS="$TARGET -Wno-error=unused-const-variable" \
  -DCMAKE_ASM_FLAGS="$TARGET" -DSTATIC_LIBS=ON -DWITH_LTO=OFF -DARCH_ID=aarch64 \
  -DWITH_UPNP=OFF -DWITH_GRPC=OFF -DWITH_MERGE_MINING_DONATION=OFF >/dev/null
cmake --build "$SRC/build" --target p2pool -j "$JOBS" >/dev/null
"$SRC/build/p2pool" --version | head -1
# Only system libraries (portable to Helios's other Macs).
if otool -L "$SRC/build/p2pool" | tail -n +2 | grep -v -e "^\s*/usr/lib/" -e "^\s*/System/Library/"; then
  echo "p2pool links a non-system library"; exit 1
fi
