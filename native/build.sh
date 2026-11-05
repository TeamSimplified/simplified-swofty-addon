#!/usr/bin/env bash
# Builds native/simplified.dll, the Grid Editor's seats on the grids the game makes from records, with Zig 0.16
# against the Swofty SDK headers in native/include (launcher 1.6.12). `build.sh test` also builds
# and runs the tests in native/test against the SDK's fake core.
set -euo pipefail
cd "$(dirname "$0")"
ZIG=${ZIG:-zig}
FLAGS=(-std=c++23 -Wall -Wextra -Wno-nullability-completeness -Iinclude)
"$ZIG" c++ -target x86_64-windows-gnu -O2 -shared -s "${FLAGS[@]}" -o simplified.dll src/grid_editor.cpp
rm -f simplified.lib simplified.pdb
echo "built $(pwd)/simplified.dll"
if [ "${1:-}" = test ]; then
    mkdir -p build
    case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
        # On Windows the SDK only talks to a registry when swofty_core.dll is loaded: a stand-in.
        "$ZIG" c++ -target x86_64-windows-gnu -shared -o build/swofty_core.dll test/core_stub.cpp
        "$ZIG" c++ -target x86_64-windows-gnu -O1 "${FLAGS[@]}" -o build/grid_editor_test.exe test/grid_editor_test.cpp
        PATH="$PWD/build:$PATH" build/grid_editor_test.exe
        ;;
    *)
        "$ZIG" c++ -O1 -fdeclspec "${FLAGS[@]}" -o build/grid_editor_test test/grid_editor_test.cpp
        build/grid_editor_test
        ;;
    esac
fi
