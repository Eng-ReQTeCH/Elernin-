use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use jni::{
    objects::{GlobalRef, JObject, JString, JValue},
    JNIEnv, JavaVM,
};
use slint::android::AndroidApp;

struct Activity {
    vm: JavaVM,
    object: GlobalRef,
}

static ACTIVITY: Mutex<Option<Arc<Activity>>> = Mutex::new(None);

pub fn init(app: &AndroidApp) -> Result<PathBuf> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast())? };
    // Retain a global reference for calls from the sync and file-opening threads.
    let object = vm
        .attach_current_thread()?
        .new_global_ref(unsafe { JObject::from_raw(app.activity_as_ptr().cast()) })?;
    *ACTIVITY.lock().expect("Android activity lock") = Some(Arc::new(Activity { vm, object }));
    with_activity(|env, object| {
        let directory = env
            .call_method(object, "libraryPath", "()Ljava/lang/String;", &[])?
            .l()?;
        let value: String = env.get_string(&JString::from(directory))?.into();
        Ok(PathBuf::from(value))
    })
}

fn with_activity<T>(call: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> Result<T>) -> Result<T> {
    let activity = ACTIVITY
        .lock()
        .expect("Android activity lock")
        .clone()
        .context("Android activity is unavailable")?;
    let mut env = activity.vm.attach_current_thread()?;
    let result = call(&mut env, activity.object.as_obj());
    if env.exception_check()? {
        env.exception_clear()?;
        anyhow::bail!("Android could not complete the request");
    }
    result
}

pub struct CredentialStore;

impl CredentialStore {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn get_password(&self) -> Result<Option<String>> {
        with_activity(|env, object| {
            let value = env
                .call_method(object, "readSignIn", "()Ljava/lang/String;", &[])?
                .l()?;
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(env.get_string(&JString::from(value))?.into()))
        })
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        with_activity(|env, object| {
            let value = JObject::from(env.new_string(password)?);
            env.call_method(
                object,
                "saveSignIn",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&value)],
            )?;
            Ok(())
        })
    }

    pub fn delete_credential(&self) -> Result<()> {
        with_activity(|env, object| {
            env.call_method(object, "forgetSignIn", "()V", &[])?;
            Ok(())
        })
    }
}

pub fn open_file(path: &Path) -> io::Result<()> {
    with_activity(|env, object| {
        let value =
            JObject::from(env.new_string(path.to_str().context("Invalid saved filename")?)?);
        env.call_method(
            object,
            "viewFile",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&value)],
        )?;
        Ok(())
    })
    .map_err(io::Error::other)
}
