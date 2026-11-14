package io.github.eng_reqtech.elearnin;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import android.webkit.MimeTypeMap;
import android.widget.Toast;

import androidx.core.content.FileProvider;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.util.Locale;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

public final class StudyActivity extends NativeActivity {
    private static final String KEY_ALIAS = "elearnin-bue";
    private static final String SIGN_IN = "sign_in";
    private static final String TRANSFORMATION = "AES/GCM/NoPadding";
    private static final int IV_LENGTH = 12;

    public String libraryPath() {
        return new File(getFilesDir(), "library").getAbsolutePath();
    }

    private SharedPreferences storage() {
        return getSharedPreferences("credentials", MODE_PRIVATE);
    }

    private static KeyStore keyStore() throws GeneralSecurityException, IOException {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        return store;
    }

    public synchronized String readSignIn() throws GeneralSecurityException, IOException {
        String saved = storage().getString(SIGN_IN, null);
        if (saved == null) return null;
        byte[] data = Base64.decode(saved, Base64.NO_WRAP);
        SecretKey key = (SecretKey) keyStore().getKey(KEY_ALIAS, null);
        if (key == null || data.length <= IV_LENGTH) throw new IOException("Saved sign-in is unavailable");
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(128, data, 0, IV_LENGTH));
        return new String(cipher.doFinal(data, IV_LENGTH, data.length - IV_LENGTH), StandardCharsets.UTF_8);
    }

    public synchronized void saveSignIn(String value) throws GeneralSecurityException, IOException {
        SecretKey key = (SecretKey) keyStore().getKey(KEY_ALIAS, null);
        if (key == null) {
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build());
            key = generator.generateKey();
        }
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(Cipher.ENCRYPT_MODE, key);
        byte[] iv = cipher.getIV();
        if (iv.length != IV_LENGTH) throw new IOException("Unexpected encryption IV length");
        byte[] encrypted = cipher.doFinal(value.getBytes(StandardCharsets.UTF_8));
        byte[] data = new byte[IV_LENGTH + encrypted.length];
        System.arraycopy(iv, 0, data, 0, IV_LENGTH);
        System.arraycopy(encrypted, 0, data, IV_LENGTH, encrypted.length);
        if (!storage().edit().putString(SIGN_IN, Base64.encodeToString(data, Base64.NO_WRAP)).commit()) {
            throw new IOException("Could not store encrypted sign-in");
        }
    }

    public synchronized void forgetSignIn() throws GeneralSecurityException, IOException {
        if (!storage().edit().remove(SIGN_IN).commit()) throw new IOException("Could not remove saved sign-in");
        keyStore().deleteEntry(KEY_ALIAS);
    }

    public void viewFile(String path) {
        File file = new File(path);
        Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", file);
        String name = file.getName();
        String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(
            name.substring(name.lastIndexOf('.') + 1).toLowerCase(Locale.ROOT));
        Intent intent = new Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, mime == null ? "application/octet-stream" : mime)
            .setClipData(ClipData.newRawUri("Study material", uri))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        runOnUiThread(() -> {
            try {
                startActivity(intent);
            } catch (ActivityNotFoundException error) {
                Toast.makeText(this, "Install a viewer for this file type.", Toast.LENGTH_LONG).show();
            }
        });
    }
}
