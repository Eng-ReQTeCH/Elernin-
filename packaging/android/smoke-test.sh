#!/usr/bin/env bash
set -euo pipefail

app=io.github.eng_reqtech.elearnin
adb install packaging/android/build/outputs/apk/debug/*-debug.apk
adb shell am start -W -n "$app/.StudyActivity"
sleep 5
adb shell pidof "$app"
adb shell run-as "$app" test -s files/library/library.sqlite3
adb logcat -d -s AndroidRuntime:E > /tmp/elearnin-crashes
if grep -q "$app" /tmp/elearnin-crashes; then
    cat /tmp/elearnin-crashes
    exit 1
fi
