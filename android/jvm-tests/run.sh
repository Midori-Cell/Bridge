#!/usr/bin/env bash
# Runs the Android app's logic tests on a plain JVM (no Android SDK needed).
# They cover the chat history (History.kt) and the phone-to-phone engine (p2p/*.kt), including a talk with the real PC
# program. Needs: a JDK, and Gradle's bundled Kotlin (GRADLE_HOME, e.g. /opt/gradle-8.14.3).
# The interop test also needs the PC test harness:   cd ../desktop && cargo build -p bridge-core --example peer_harness
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$here/../app/src/main/java/dev/bridge/phone"
G="${GRADLE_HOME:-/opt/gradle-8.14.3}/lib"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
CP="$(ls "$G"/kotlin-stdlib-*.jar "$G"/kotlinx-serialization-core-jvm-*.jar "$G"/kotlinx-serialization-json-jvm-*.jar "$G"/kotlinx-coroutines-core-jvm-*.jar | grep -v sources | tr '\n' ':')"
compiler="$G/*"
run() { # compile the app sources plus one test file, then run it
  local t="$1"; shift
  rm -rf "$out"/*
  java -cp "$compiler" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -cp "$CP" -d "$out" \
    "$app/Models.kt" "$app/History.kt" "$app/p2p/Identity.kt" "$app/p2p/Tls.kt" "$app/p2p/Net.kt" "$app/p2p/PeerEngine.kt" "$here/$t.kt" 2>&1 | grep -v "^warning" || true
  echo "== $t"; java -cp "$out:$CP" "${t}Kt" "$@"
}
for t in HistoryTest NetTest EngineTest; do run "$t"; done
harness="${1:-$here/../../desktop/target/debug/examples/peer_harness}"
if [ -x "$harness" ]; then run InteropTest "$harness"; else echo "(skipping interop test: $harness not built)"; fi
