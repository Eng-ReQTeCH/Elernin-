# Elearnin’

Elearnin’ downloads study materials from BUE Learn1 into a local library. Open the app, choose a module, then click a lecture or tutorial to open the saved file in your default viewer.

Sync opens course sections and Moodle week tiles in the background, including downloadable files in linked Moodle folders. The module view has **Lectures**, **Tutorials**, and **Everything else** columns. Files already saved on your device open while sync runs. The header includes a theme toggle, and the mouse Back button returns from a module to the module grid.

## Run it

Install a current stable Rust toolchain. On Windows, install Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK. On Linux, install a C compiler, `pkg-config`, D-Bus development headers, a running Secret Service keyring such as GNOME Keyring or KWallet with Secret Service enabled, and `xdg-open`.

```sh
cargo run --release --locked
```

On the first launch, click **Connect BUE**, enter your Learn1 username and password, then click **Save and sync**. Enter credentials directly in the app. Elearnin’ stores them in Windows Credential Manager or the Linux desktop keyring. **Forget sign-in** removes them and leaves the saved library available.

Elearnin’ syncs on launch and every 15 minutes while it’s open. It uses Moodle’s mobile API when available and can fall back to the ordinary Learn1 login and course pages. If the session expires, it signs in again with the saved credentials. If the university rejects the credentials or requires an interactive check, the sync status explains what happened.

Each device has its own offline copy. The first version syncs downloadable course files. External streams and activities such as quizzes or submissions are handled through the university portal. Android packaging, credential storage, and the Android file viewer handoff are future work.

## Verify locally

```sh
cargo test --locked
```

For a live check, let one module sync, close and reopen the app to start a fresh session, and confirm it syncs without prompting. Then disconnect from the internet and open a downloaded lecture from its module.
