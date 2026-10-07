#!/usr/bin/env bash
# Talk to the assistant on this machine: microphone -> speech to text -> the
# assistant -> text to speech -> loudspeaker.
#
# Downloads the speech models app/config/settings.toml names (about 800 MB on
# first use, into models/speech) and starts the app with --local-stt-mode: a
# small window with the call id and a mute button. Like --prompt-loop it reads
# .env, then .env.stdin, and needs db, redis and the model server reachable
# from this machine.
#
# Usage: ./run-local-stt.sh
set -euo pipefail

cd "$(dirname "$0")"

SETTINGS=app/config/settings.toml
MODELS=models/speech
RELEASES=https://github.com/k2-fsa/sherpa-onnx/releases/download

if [ ! -f .env.stdin ]; then
    echo "error: .env.stdin is missing (localhost values for db, redis and the model server)" >&2
    exit 1
fi

# named <key>: the file or folder under models/speech that <key> is set to in
# the settings. Empty for a path somewhere else, which is then not downloaded.
named() {
    sed -n "s|^$1 *= *\"$MODELS/\([^\"/]*\)\".*|\1|p" "$SETTINGS"
}

# fetch_archive <release tag> <name>: models/speech/<name>/ from <name>.tar.bz2
fetch_archive() {
    local tag=$1 name=$2

    if [ -z "$name" ] || [ -d "$MODELS/$name" ]; then
        return
    fi

    echo "==> Downloading $name"
    curl -fL --retry 3 -o "$MODELS/$name.tar.bz2" "$RELEASES/$tag/$name.tar.bz2"
    tar -xjf "$MODELS/$name.tar.bz2" -C "$MODELS"
    rm "$MODELS/$name.tar.bz2"
}

mkdir -p "$MODELS"

# [stt_settings]: the voice activity detector is one file, the recognizer a folder
VAD="$(named vad_model)"
if [ -n "$VAD" ] && [ ! -f "$MODELS/$VAD" ]; then
    echo "==> Downloading $VAD"
    curl -fL --retry 3 -o "$MODELS/$VAD.part" "$RELEASES/asr-models/$VAD"
    mv "$MODELS/$VAD.part" "$MODELS/$VAD"
fi

fetch_archive asr-models "$(named model_dir)"

# [tts_settings.voices]: one folder per language
for voice in $(named '[a-z][a-z]'); do
    fetch_archive tts-models "$voice"
done

echo "==> Starting the app in local STT mode"
exec cargo run -p app --features local-stt -- --local-stt-mode
