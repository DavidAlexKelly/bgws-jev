#!/usr/bin/env bash
# Pitch's own Federate Protocol client against our mock RTI: two copies of
# FedProClient's Java Chat sample join through `bgws mock-rti`, see each other
# join, swap public and private (directed) messages, and see each other leave.
#
#   tools/fedpro-chat-check.sh <FedProClient checkout> [bgws binary]
#
# Needs a JDK (javac, java). Pitch's client is compiled with javac from its
# committed sources and the jars in java/lib, so nothing is downloaded.
set -euo pipefail
FEDPRO=$(cd "${1:?usage: $0 <FedProClient checkout> [bgws binary]}" && pwd)
BGWS=$(realpath "${2:-target/release/bgws}")
J="$FEDPRO/java"
WORK=$(mktemp -d)
PORT=${PORT:-15164}
trap 'kill "${MOCK:-0}" 2>/dev/null || true; rm -rf "$WORK"' EXIT

find "$J/session" "$J/client_common" "$J/client" "$J/api/HLA1516-2025" "$J/protobuf/src" "$J/sample/src" -name '*.java' -not -path '*/test/*' > "$WORK/sources"
javac -nowarn -d "$WORK/classes" -cp "$J/lib/*" @"$WORK/sources" 2>&1 | grep -v '^Note:' || true
CP="$WORK/classes:$J/client/resources:$J/lib/*"

"$BGWS" mock-rti --listen "127.0.0.1:$PORT" > "$WORK/mock.log" 2>&1 &
MOCK=$!
sleep 1
cd "$J/sample"   # Chat reads its FOM, Chat-hla4.xml, from here
chat() { java -cp "$CP" se.pitch.oss.chat1516_4.Chat "localhost:$PORT"; }
( echo alice; sleep 5; echo "hello from alice"; sleep 3; echo "bob: private from alice"; sleep 4; echo .; sleep 3; echo ) | timeout 40 bash -c "$(declare -f chat); CP='$CP' PORT=$PORT chat" > "$WORK/alice.log" 2>&1 &
A=$!
sleep 2
( echo bob; sleep 4; echo "hello from bob"; sleep 3; echo "alice: private from bob"; sleep 4; echo .; sleep 3; echo ) | timeout 40 bash -c "$(declare -f chat); CP='$CP' PORT=$PORT chat" > "$WORK/bob.log" 2>&1 &
B=$!
wait $A $B || true

fail=0
want() { grep -qF "$2" "$WORK/$1.log" || { echo "MISSING in $1: $2"; fail=1; }; }
want alice "[bob has joined]"
want alice "bob: hello from bob"
want alice "Private message from bob: private from bob"
want bob "[alice has joined]"
want bob "alice: hello from alice"
want bob "Private message from alice: private from alice"
want bob "[alice has left]"
if grep -q "SEVERE\|Exception" "$WORK/alice.log" "$WORK/bob.log"; then echo "errors in the Chat logs"; fail=1; fi
if [ $fail -ne 0 ]; then
  echo "--- alice"; cat "$WORK/alice.log"; echo "--- bob"; cat "$WORK/bob.log"; echo "--- mock"; cat "$WORK/mock.log"
  exit 1
fi
echo "Pitch's Chat sample works against bgws mock-rti"
