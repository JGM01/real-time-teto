#!/bin/zsh
# Explicitly run from the repository root after authorizing installation on this device.
set -euo pipefail
: "${TETO_PHONE_DEVICE:?Set the authorized devicectl device identifier}"
phone_file=$(python3 -c 'import re;print(re.search(r"pub const PHONE_FILE: &str = \"([^\"]+)\"",open("teto-llm-protocol/src/policy.rs").read()).group(1))')
phone_sha=$(python3 -c 'import re;print(re.search(r"pub const PHONE_SHA256: &str = \"([^\"]+)\"",open("teto-llm-protocol/src/policy.rs").read()).group(1))')
if [[ ! -f "models/$phone_file" ]]; then
  curl --fail --location --retry 2 --connect-timeout 20 --max-time 1800 --continue-at - --output "models/$phone_file.partial" "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/main/$phone_file"
  python3 - "$phone_file" "$phone_sha" <<'PY'
from pathlib import Path
import hashlib,sys
p=Path('models')/(sys.argv[1]+'.partial')
h=hashlib.sha256()
with p.open('rb') as f:
    while data:=f.read(1024*1024): h.update(data)
assert h.hexdigest()==sys.argv[2], 'Phone model checksum mismatch'
p.rename(p.with_suffix(''))
PY
fi
make phone-build
xcrun devicectl device info details --device "$TETO_PHONE_DEVICE" --timeout 60 --json-output phone-spike/target/device.json
phone_udid=$(python3 -c 'import json;print(json.load(open("phone-spike/target/device.json"))["result"]["hardwareProperties"]["udid"])')
xcodebuild -project phone-spike/TetoPhoneSpike.xcodeproj -scheme TetoPhoneSpike -configuration Release -destination "id=$phone_udid" -derivedDataPath phone-spike/target/xcode -allowProvisioningUpdates -allowProvisioningDeviceRegistration build
xcrun devicectl device install app --device "$TETO_PHONE_DEVICE" --timeout 120 phone-spike/target/xcode/Build/Products/Release-iphoneos/TetoPhoneSpike.app
xcrun devicectl device copy to --device "$TETO_PHONE_DEVICE" --timeout 600 --source "models/$phone_file" --destination "Documents/$phone_file" --domain-type appDataContainer --domain-identifier org.teto.PhoneSpike
xcrun devicectl device process launch --device "$TETO_PHONE_DEVICE" --timeout 120 --activate --terminate-existing org.teto.PhoneSpike --autorun
