#!/bin/bash
# Reproducible monerod build for the managed Monero node (p2pool without the Monero GUI).
# Pinned release commit, both Mac architectures, every dependency built from source by Monero's own
# contrib/depends (the system official releases use), linked statically: `otool -L` shows only
# /usr/lib and /System. Nothing from Homebrew, nothing installed outside the repo; sources, caches
# and build trees all stay under vendor/monero/.
#
# depends on a darwin build host uses Xcode's clang (builders/darwin.mk). We override:
#   - native_packages (ccache: its cache dir is in ~; not needed),
#   - darwin_packages (drop hidapi/libusb/protobuf: hardware-wallet support, not used by monerod),
#   - darwin_CC/CXX (add -arch, so an x86_64 build works on an arm64 Mac),
#   - OSX_MIN_VERSION (11.3 arm64 floor, 10.15 Intel floor, roadmap Stage 3),
#   - boost toolset darwin -> clang (clang-darwin): Boost 1.69's darwin.jam passes
#     -fcoalesce-templates, which current Apple clang rejects.
# The generated depends toolchain.cmake assumes a Linux cross build (native/bin/clang, native/SDK),
# so a small wrapper toolchain includes it and points the compiler and SDK at Xcode's.
# -ffile-prefix-map: no repo or home path (the username) in the shipped binary.
#
# Usage: tools/build-monerod.sh [arm64|x86_64|all]   (default: all)
# Outputs: vendor/monero/build-out/arm64/monerod, vendor/monero/build-out/x86_64/monerod
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WHAT="${1:-all}"
SRC="$ROOT/vendor/monero"
TAG="v0.18.5.3"                                      # "Fluorine Fermi", signed tag 2026-10-02
PIN="22578c3f7d7b4b6dd85ff7daa42a827d97cc53d0"
JOBS="$(sysctl -n hw.ncpu)"
MAP="-ffile-prefix-map=$ROOT=. -ffile-prefix-map=$HOME=~"
# Boost 1.69 MPL casts -1 into enums in constant expressions; clang 16+ makes that an error by
# default (fixed upstream in Boost 1.81). Same code as before clang 16, so downgrade it.
NOENUM="-Wno-enum-constexpr-conversion"
export SDKROOT="$(xcrun --show-sdk-path)"
export CCACHE_DISABLE=1
CC_BIN="$(xcrun -f clang)"; CXX_BIN="$(xcrun -f clang++)"; AR_BIN="$(xcrun -f ar)"

if [ ! -d "$SRC/.git" ]; then
  rm -rf "$SRC"   # pruned by tools/prune-vendor.sh: only finished binaries were left
  git clone --filter=blob:none https://github.com/monero-project/monero.git "$SRC"
fi
if [ "$(git -C "$SRC" rev-parse HEAD)" != "$PIN" ]; then
  git -C "$SRC" fetch --depth 1 origin "$PIN"
  git -C "$SRC" checkout --quiet "$PIN"
fi
git -C "$SRC" submodule update --init --recursive --quiet

# Build-host tools: the unbound and sodium recipes run `autoconf`, which macOS lacks (and its m4
# 1.4.6 is too old for autoconf). Both from pinned GNU tarballs into vendor/, never installed.
TOOLS="$SRC/build-out/hosttools"
export PATH="$TOOLS/bin:$PATH"
host_tool() {   # name version url sha256
  local name="$1" ver="$2" url="$3" sha="$4" tgz="$TOOLS/src/$1-$2.tar.gz"
  [ -x "$TOOLS/bin/$name" ] && return 0
  mkdir -p "$TOOLS/src"
  [ -f "$tgz" ] || curl -sSfL -o "$tgz" "$url"
  echo "$sha  $tgz" | shasum -a 256 -c --quiet
  rm -rf "$TOOLS/src/$name-$ver" && tar -xzf "$tgz" -C "$TOOLS/src"
  ( cd "$TOOLS/src/$name-$ver" && ./configure --prefix="$TOOLS" >/dev/null \
    && make -j "$JOBS" >/dev/null && make install >/dev/null )
}
host_tool m4 1.4.19 https://ftp.gnu.org/gnu/m4/m4-1.4.19.tar.gz \
  3be4a26d825ffdfda52a56fc43246456989a3630093cced3fbddf4771ee58a70
host_tool autoconf 2.72 https://ftp.gnu.org/gnu/autoconf/autoconf-2.72.tar.gz \
  afb181a76e1ee72832f6581c0eddf8df032b83e2e0239ef79ebedc4467d92d6e

build_one() {   # $1 = arm64 | x86_64
  local arch="$1" host min target out prefix
  case "$arch" in
    arm64)  host=aarch64-apple-darwin; min=11.3 ;;
    x86_64) host=x86_64-apple-darwin;  min=10.15 ;;
    *) echo "unknown arch $arch" >&2; exit 2 ;;
  esac
  target="$arch-apple-macos$min"
  out="$SRC/build-out/$arch"
  prefix="$SRC/contrib/depends/$host"
  mkdir -p "$out"

  # 1. Dependencies from source (boost, openssl, zeromq, libiconv, expat, unbound, ncurses,
  #    readline, sodium) into contrib/depends/<host>/. Cached by package id: a rerun is a no-op.
  #    The boost and zeromq recipes replace the host CXXFLAGS with their own, so they get ours
#    explicitly.
  #    HOST_ID_SALT hashes these overrides into every package id, so a changed override
  #    never reuses a stale cached package or work dir.
  local -a overrides=(
    native_packages= darwin_packages="ncurses readline sodium"
    OSX_MIN_VERSION="$min"
    darwin_CC="$CC_BIN -arch $arch -mmacosx-version-min=$min"
    darwin_CXX="$CXX_BIN -arch $arch -mmacosx-version-min=$min -stdlib=libc++"
    darwin_CFLAGS="-pipe $MAP $NOENUM"
    boost_toolset_darwin=clang boost_archiver_darwin="$AR_BIN"
    boost_config_opts_darwin="--toolset=clang runtime-link=shared"
    boost_cxxflags="-std=c++11 $MAP $NOENUM"
    zeromq_cxxflags="-std=c++11 $MAP -Wno-deprecated-declarations"   # its -Werror vs. sprintf
    # OpenSSL and unbound bake their runtime dirs (prefix/etc/...) into the binary: point them at
    # /var/empty (always empty, root-owned) so no repo path (the username) ships. OpenSSL never
    # autoloads an openssl.cnf. The install layout under contrib/depends/<host>/ is unchanged.
    openssl_config_opts_darwin=no-autoload-config
    'openssl_build_cmds=$(MAKE) build_libs OPENSSLDIR=/var/empty/xmrange-openssl ENGINESDIR=/var/empty/xmrange-openssl/engines-3 MODULESDIR=/var/empty/xmrange-openssl/ossl-modules'
    unbound_config_opts_darwin=--sysconfdir=/var/empty/xmrange-unbound
  )
  local salt; salt="xmrange-$(printf '%s\n' "${overrides[@]}" | shasum -a 256 | cut -c1-16)"
  make -C "$SRC/contrib/depends" -j "$JOBS" HOST="$host" HOST_ID_SALT="$salt" \
    "${overrides[@]}" >"$out/depends.log" 2>&1 \
    || { tail -40 "$out/depends.log"; exit 1; }

  # 2. Wrapper toolchain: the depends one, with Xcode's compiler and SDK.
  cat >"$out/toolchain.cmake.new" <<EOF
include("$prefix/share/toolchain.cmake")
set(CMAKE_C_COMPILER "$CC_BIN")
set(CMAKE_CXX_COMPILER "$CXX_BIN")
set(CMAKE_C_COMPILER_TARGET "$target")
set(CMAKE_CXX_COMPILER_TARGET "$target")
set(CMAKE_ASM_COMPILER_TARGET "$target")
set(CMAKE_ASM-ATT_COMPILER_TARGET "$target")
set(CMAKE_C_FLAGS_INIT "$MAP")
set(CMAKE_CXX_FLAGS_INIT "$MAP $NOENUM -stdlib=libc++")
set(CMAKE_OSX_SYSROOT "$SDKROOT")
set(CMAKE_OSX_DEPLOYMENT_TARGET "$min")
set(CMAKE_OSX_ARCHITECTURES "$arch")
set(CMAKE_FIND_ROOT_PATH "$prefix" "$SDKROOT" /usr)
EOF
  # Replace only on change: a new mtime makes CMake reconfigure and rebuild everything.
  if cmp -s "$out/toolchain.cmake.new" "$out/toolchain.cmake"; then rm "$out/toolchain.cmake.new"
  else mv "$out/toolchain.cmake.new" "$out/toolchain.cmake"; fi

  # 3. monerod only (target "daemon"): no wallet, GUI deps, tests or docs. MD/BSD_LIBRARY: the
  #    ZMQ block links any libmd/libbsd it finds; the current SDK has /usr/lib/libmd.dylib, which
  #    older macOS lacks (dyld would refuse to start), and monerod uses none of it.
  cmake -S "$SRC" -B "$out/build" -DCMAKE_TOOLCHAIN_FILE="$out/toolchain.cmake" \
    -DCMAKE_BUILD_TYPE=Release -DUSE_CCACHE=OFF -DBUILD_DOCUMENTATION=OFF -DBUILD_TESTS=OFF \
    -DMANUAL_SUBMODULES=ON -DUSE_DEVICE_TREZOR=OFF \
    -DMD_LIBRARY=MD_LIBRARY-NOTFOUND -DBSD_LIBRARY=BSD_LIBRARY-NOTFOUND >"$out/cmake.log" 2>&1 \
    || { tail -40 "$out/cmake.log"; exit 1; }
  cmake --build "$out/build" --target daemon -j "$JOBS" >"$out/build.log" 2>&1 \
    || { grep -m20 -E 'error|Error' "$out/build.log"; exit 1; }
  cp "$out/build/bin/monerod" "$out/monerod"
  strip "$out/monerod"
  # CMake leaves an LC_RPATH to depends' lib folder (a repo path with the user name). Everything is
  # linked statically, so it is dead: delete every rpath, then re-sign ad hoc (the edit breaks the
  # linker's signature, and arm64 runs nothing unsigned).
  while read -r rp; do
    install_name_tool -delete_rpath "$rp" "$out/monerod" 2>/dev/null
  done < <(otool -l "$out/monerod" | awk '/LC_RPATH/{getline; getline; print $2}')
  codesign --force --sign - "$out/monerod" >/dev/null 2>&1
  lipo -info "$out/monerod"
  if otool -L "$out/monerod" | tail -n +2 | grep -vE '^\s+(/usr/lib/|/System/)'; then
    echo "non-system library linked" >&2; exit 1
  fi
  # Every printable run of every byte (tools/check-release.sh's scan): `strings` skips load commands.
  local text; text="$(LC_ALL=C tr -c '[:print:]' '\n' < "$out/monerod" | grep -E '.{4,}')"
  if grep -F -e HouseOfHelios -e "$HOME" <<<"$text" >/dev/null; then
    echo "repo or home path in $out/monerod" >&2; exit 1
  fi
  "$out/monerod" --version 2>/dev/null || arch -x86_64 "$out/monerod" --version
}

case "$WHAT" in
  arm64|x86_64) build_one "$WHAT" ;;
  all) build_one arm64; build_one x86_64 ;;
  *) echo "usage: $0 [arm64|x86_64|all]" >&2; exit 2 ;;
esac
