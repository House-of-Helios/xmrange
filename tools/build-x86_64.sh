#!/bin/bash
# Stage 3, first module: x86_64 (Intel) builds of the three bundled helpers (xmrig-mo with our
# patches, p2pool, Arti), plus a `lipo -create` universal binary of each. Same pinned sources as the
# arm64 scripts (run tools/build-xmrig.sh, build-p2pool.sh and build-arti.sh first: they clone, pin
# and patch). Deployment target 10.15 (the Tauri floor, roadmap Stage 3).
#
# Dependencies come from pinned upstream source, built into vendor/deps-x86_64/ and linked
# statically: `otool -L` shows only /usr/lib and /System. Nothing from Homebrew, nothing installed
# outside the repo. -ffile-prefix-map / --remap-path-prefix: no repo or home path (the username) in
# any shipped binary (tools/check-release.sh needles).
#
# Usage: tools/build-x86_64.sh [xmrig|p2pool|arti|all]   (default: all)
# Outputs (thin x86_64):  vendor/xmrig-mo/build-x86_64/xmrig
#                         vendor/p2pool-x86_64/build/p2pool
#                         vendor/arti/target/x86_64-apple-darwin/release/arti
#         (universal):    vendor/universal/{xmrig,p2pool,arti}
set -euo pipefail
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WHAT="${1:-all}"
JOBS="$(sysctl -n hw.ncpu)"
MIN="10.15"
DEPS="$ROOT/vendor/deps-x86_64"      # prefix: include/ + lib/ (XMRIG_DEPS layout)
DSRC="$DEPS/src"
UNI="$ROOT/vendor/universal"
MAP="-ffile-prefix-map=$ROOT=. -ffile-prefix-map=$HOME=~"

# Pinned dependency sources (git tag + the commit it must resolve to).
UV_TAG="v1.53.0";          UV_PIN="840404ce8ba7cc0204be52389a6cfff9f2c90fb6"
OSSL_TAG="openssl-3.5.9";  OSSL_PIN="45e844fa2a14ec92d146bd8f5778ac130b6625fb"   # 3.5 LTS

fetch() {  # fetch <dir> <url> <tag> <pin>
  [ -d "$1/.git" ] || git clone --quiet --depth 1 --branch "$3" "$2" "$1" 2>/dev/null
  [ "$(git -C "$1" rev-parse HEAD)" = "$4" ] || { echo "$1 is not at $3 ($4)" >&2; exit 1; }
}

system_only() {  # system_only <binary>: fail when it links anything outside /usr/lib and /System
  if otool -L "$1" | tail -n +2 | grep -v -e "^\s*/usr/lib/" -e "^\s*/System/Library/"; then
    echo "$1 links a non-system library" >&2; exit 1
  fi
}

universal() {  # universal <arm64 binary> <x86_64 binary> <name>
  [ -x "$1" ] || { echo "$1 not built: run the arm64 build script first" >&2; exit 1; }
  mkdir -p "$UNI"
  lipo -create "$1" "$2" -output "$UNI/$3"
  lipo -info "$UNI/$3"
}

deps_xmrig() {
  mkdir -p "$DSRC" "$DEPS/lib" "$DEPS/include"
  # libuv
  if [ ! -f "$DEPS/lib/libuv.a" ]; then
    fetch "$DSRC/libuv" https://github.com/libuv/libuv.git "$UV_TAG" "$UV_PIN"
    cmake -S "$DSRC/libuv" -B "$DEPS/build/libuv" -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 \
      -DCMAKE_OSX_DEPLOYMENT_TARGET=$MIN -DCMAKE_C_FLAGS="$MAP" -DBUILD_TESTING=OFF -DLIBUV_BUILD_SHARED=OFF \
      -DCMAKE_INSTALL_PREFIX="$DEPS" -DCMAKE_INSTALL_LIBDIR=lib >/dev/null
    cmake --build "$DEPS/build/libuv" -j "$JOBS" >/dev/null
    cmake --install "$DEPS/build/libuv" >/dev/null
  fi
  # OpenSSL (MoneroOcean mode mines over TLS: config.rs tls: true). A fake prefix under /var/empty
  # (always empty, root-owned) so no repo path is baked in as OPENSSLDIR/ENGINESDIR/MODULESDIR;
  # no-autoload-config: never reads an openssl.cnf at runtime. no-filenames instead of
  # -ffile-prefix-map: OpenSSL bakes its compiler flags into the library, so a path in a flag leaks.
  if [ ! -f "$DEPS/lib/libssl.a" ]; then
    fetch "$DSRC/openssl" https://github.com/openssl/openssl.git "$OSSL_TAG" "$OSSL_PIN"
    local B="$DEPS/build/openssl" P="/var/empty/xmrange-openssl"
    rm -rf "$B" "$DEPS/stage" && mkdir -p "$B"
    (cd "$B" && "$DSRC/openssl/Configure" darwin64-x86_64-cc no-shared no-module no-tests no-docs no-zlib no-comp \
      no-autoload-config no-filenames --prefix="$P" --openssldir="$P/ssl" --libdir=lib -mmacosx-version-min=$MIN >/dev/null \
      && make -j"$JOBS" build_libs >/dev/null && make install_dev DESTDIR="$DEPS/stage" >/dev/null)
    cp -R "$DEPS/stage$P/include/openssl" "$DEPS/include/"
    cp "$DEPS/stage$P/lib/libssl.a" "$DEPS/stage$P/lib/libcrypto.a" "$DEPS/lib/"
    rm -rf "$DEPS/stage"
  fi
}

build_xmrig() {
  local SRC="$ROOT/vendor/xmrig-mo" PATCH="$ROOT/patches/xmrig-mo-xmrange.patch"
  [ -d "$SRC/.git" ] || { echo "vendor/xmrig-mo missing: run tools/build-xmrig.sh first" >&2; exit 1; }
  [ "$(git -C "$SRC" rev-parse HEAD)" = "0ce9664d3f540b72624e3733a592ddfdf02662a5" ] || { echo "xmrig-mo is not at the pin" >&2; exit 1; }
  git -C "$SRC" apply --reverse --check "$PATCH" 2>/dev/null || { echo "xmrig-mo patches not applied: run tools/build-xmrig.sh" >&2; exit 1; }
  deps_xmrig
  # CMAKE_SYSTEM_NAME + PROCESSOR: xmrig picks its x86 code paths (RandomX JIT, asm) from the processor.
  cmake -S "$SRC" -B "$SRC/build-x86_64" -DCMAKE_BUILD_TYPE=Release -DCMAKE_SYSTEM_NAME=Darwin \
    -DCMAKE_SYSTEM_PROCESSOR=x86_64 -DCMAKE_OSX_ARCHITECTURES=x86_64 -DCMAKE_OSX_DEPLOYMENT_TARGET=$MIN \
    -DCMAKE_C_FLAGS="$MAP" -DCMAKE_CXX_FLAGS="$MAP" -DWITH_HWLOC=OFF -DXMRIG_DEPS="$DEPS" >/dev/null
  cmake --build "$SRC/build-x86_64" -j "$JOBS" >/dev/null
  system_only "$SRC/build-x86_64/xmrig"
  universal "$SRC/build/xmrig" "$SRC/build-x86_64/xmrig" xmrig
}

build_p2pool() {
  local ORIG="$ROOT/vendor/p2pool" SRC="$ROOT/vendor/p2pool-x86_64"
  # CMAKE_OSX_ARCHITECTURES: on an Apple Silicon host CMake otherwise adds -arch arm64, which wins over -target.
  local OSXA="-DCMAKE_OSX_ARCHITECTURES=x86_64"
  local T="-target x86_64-apple-macos$MIN -ffile-prefix-map=$SRC=p2pool -ffile-prefix-map=$HOME=~"
  [ "$(git -C "$ORIG" rev-parse HEAD 2>/dev/null)" = "abc833598f80d7a0345b6020ced90f19957d7a7c" ] \
    || { echo "vendor/p2pool missing or off pin: run tools/build-p2pool.sh first" >&2; exit 1; }
  # p2pool's CMakeLists links its static deps from fixed in-tree paths (external/src/*/), and the
  # arm64 recipe builds curl in-tree. So x86_64 gets its own tree: an APFS clone (cp -c, no extra
  # disk) with the arm64 build outputs dropped and curl replaced by a pristine copy of its commit.
  if [ ! -f "$SRC/CMakeLists.txt" ]; then   # missing, or pruned to its binary by tools/prune-vendor.sh
    rm -rf "$SRC" && cp -c -R "$ORIG" "$SRC"
    rm -rf "$SRC/build" "$SRC/external/src/libuv/build" "$SRC/external/src/libzmq/build" "$SRC/external/src/curl"
    mkdir -p "$SRC/external/src/curl" && git -C "$ORIG/external/src/curl" archive HEAD | tar -x -C "$SRC/external/src/curl"
  fi
  # The arm64 recipe, with the x86_64 target (curl: HTTP only, no TLS: it only talks to monerod on 127.0.0.1).
  (cd "$SRC/external/src/curl" && [ -f lib/libcurl.a ] || {
    cmake . -DCMAKE_BUILD_TYPE=Release $OSXA -DCMAKE_C_FLAGS="-Os $T" -DCURL_DROP_UNUSED=ON -DCURL_DISABLE_TYPECHECK=ON \
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
    cmake .. -DCMAKE_BUILD_TYPE=Release $OSXA -DCMAKE_C_FLAGS="-Os $T" -DBUILD_TESTING=OFF -DLIBUV_BUILD_SHARED=OFF -DENABLE_CLANG_TIDY=OFF >/dev/null
    make -j"$JOBS" >/dev/null; })
  (mkdir -p "$SRC/external/src/libzmq/build" && cd "$SRC/external/src/libzmq/build" && [ -f lib/libzmq.a ] || {
    cmake .. -DCMAKE_BUILD_TYPE=Release $OSXA -DCMAKE_C_FLAGS="-Os $T" -DCMAKE_CXX_FLAGS="-Os $T" -DWITH_LIBSODIUM=OFF \
      -DWITH_LIBBSD=OFF -DBUILD_TESTS=OFF -DWITH_DOCS=OFF -DENABLE_DRAFTS=OFF -DBUILD_SHARED=OFF >/dev/null
    make -j"$JOBS" >/dev/null; })
  mkdir -p "$SRC/build"
  cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release $OSXA -DCMAKE_C_FLAGS="$T" \
    -DCMAKE_CXX_FLAGS="$T -Wno-error=unused-const-variable" -DCMAKE_ASM_FLAGS="$T" -DSTATIC_LIBS=ON -DWITH_LTO=OFF \
    -DARCH_ID=x86_64 -DWITH_UPNP=OFF -DWITH_GRPC=OFF -DWITH_MERGE_MINING_DONATION=OFF >/dev/null
  cmake --build "$SRC/build" --target p2pool -j "$JOBS" >/dev/null
  system_only "$SRC/build/p2pool"
  universal "$ORIG/build/p2pool" "$SRC/build/p2pool" p2pool
}

build_arti() {
  local SRC="$ROOT/vendor/arti"
  [ "$(git -C "$SRC" rev-parse HEAD 2>/dev/null)" = "a71097fdf7b141b56d1eb3709628ee38d232c9d1" ] \
    || { echo "vendor/arti missing or off pin: run tools/build-arti.sh first" >&2; exit 1; }
  (cd "$SRC" && MACOSX_DEPLOYMENT_TARGET=$MIN RUSTFLAGS="--remap-path-prefix=$SRC=arti --remap-path-prefix=$HOME=~" \
    CFLAGS_x86_64_apple_darwin="$MAP" cargo build --release --locked -p arti --target x86_64-apple-darwin >/dev/null 2>&1)
  system_only "$SRC/target/x86_64-apple-darwin/release/arti"
  universal "$SRC/target/release/arti" "$SRC/target/x86_64-apple-darwin/release/arti" arti
}

case "$WHAT" in
  xmrig) build_xmrig ;;
  p2pool) build_p2pool ;;
  arti) build_arti ;;
  all) build_xmrig; build_p2pool; build_arti ;;
  *) echo "usage: $0 [xmrig|p2pool|arti|all]" >&2; exit 2 ;;
esac
