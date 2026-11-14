#[cfg(not(target_os = "android"))]
mod desktop {
    use std::path::Path;

    pub struct CredentialStore(keyring::Entry);

    impl CredentialStore {
        pub fn new() -> anyhow::Result<Self> {
            Ok(Self(keyring::Entry::new(
                "Elearnin BUE Learn1",
                "saved-sign-in",
            )?))
        }

        pub fn get_password(&self) -> anyhow::Result<Option<String>> {
            match self.0.get_password() {
                Ok(password) => Ok(Some(password)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.into()),
            }
        }

        pub fn set_password(&self, password: &str) -> anyhow::Result<()> {
            Ok(self.0.set_password(password)?)
        }

        pub fn delete_credential(&self) -> anyhow::Result<()> {
            match self.0.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
    }

    pub fn open_file(path: &Path) -> std::io::Result<()> {
        open::that_detached(path)
    }
}

#[cfg(not(target_os = "android"))]
pub use desktop::*;

#[cfg(target_os = "android")]
#[path = "platform_android.rs"]
mod android;

#[cfg(target_os = "android")]
pub use android::*;
