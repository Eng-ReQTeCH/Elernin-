#!/usr/bin/env bash
set -euo pipefail

app=io.github.eng_reqtech.elearnin
adb install packaging/android/build/outputs/apk/debug/*-debug.apk
adb shell am start -W -n "$app/.StudyActivity"
sleep 5
adb shell pidof "$app"
adb shell run-as "$app" test -s files/library/library.sqlite3
mkdir -p target/android-smoke
for mode in gestural threebutton; do
    adb shell cmd overlay enable-exclusive --category "com.android.internal.systemui.navbar.$mode"
    adb shell am start -W -n "$app/.StudyActivity"
    sleep 3
    adb exec-out screencap -p > "target/android-smoke/$mode.png"
done
adb logcat -d -s AndroidRuntime:E > /tmp/elearnin-crashes
if grep -q "$app" /tmp/elearnin-crashes; then
    cat /tmp/elearnin-crashes
    exit 1
fi
