package io.mirelay.android;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.FileNotFoundException;
import java.io.IOException;

/** Test-APK process has no access to the target APK's Kotlin runtime. Keep this
 * synthetic external provider plain Java; it never reads arbitrary local files. */
public final class FixtureProvider extends ContentProvider {
    @Override public boolean onCreate() { return true; }
    @Override public android.os.Bundle call(String method, String arg, android.os.Bundle extras) {
        if ("auto".equals(method)) return AutoFixtureProvider.control(getContext(), arg, extras == null ? new android.os.Bundle() : extras);
        return super.call(method, arg, extras);
    }
    @Override public String getType(Uri uri) { return "application/octet-stream"; }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String sort) {
        String[] columns = projection != null ? projection : new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor cursor = new MatrixCursor(columns);
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) {
                String name = uri.getQueryParameter("name"); row[i] = name != null ? name : "fixture.bin";
            } else if (OpenableColumns.SIZE.equals(columns[i])) {
                String size = uri.getQueryParameter("declared"); row[i] = size != null ? Long.valueOf(size) : null;
            }
        }
        cursor.addRow(row); return cursor;
    }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if ("/denied".equals(uri.getPath())) throw new FileNotFoundException("Fixture access revoked");
        if (!"r".equals(mode)) throw new FileNotFoundException("Read only");
        String sizeArg = uri.getQueryParameter("size");
        int size = sizeArg == null ? 4096 : Integer.parseInt(sizeArg);
        if (size < 0 || size > 101 * 1024 * 1024) throw new FileNotFoundException("Invalid fixture size");
        final ParcelFileDescriptor[] pipe;
        try { pipe = ParcelFileDescriptor.createPipe(); }
        catch (IOException error) { throw new FileNotFoundException(error.getMessage()); }
        new Thread(() -> {
            try (ParcelFileDescriptor.AutoCloseOutputStream output = new ParcelFileDescriptor.AutoCloseOutputStream(pipe[1])) {
                byte[] buffer = new byte[65536];
                int written = 0;
                while (written < size) {
                    int count = Math.min(buffer.length, size - written);
                    for (int i = 0; i < count; i++) buffer[i] = (byte)((written + i) % 251);
                    output.write(buffer, 0, count); written += count;
                }
            } catch (IOException ignored) { /* Reader intentionally rejected/cancelled a fixture. */ }
        }, "fixture-provider").start();
        return pipe[0];
    }
    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException("Read only"); }
    @Override public int delete(Uri uri, String selection, String[] args) { throw new UnsupportedOperationException("Read only"); }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] args) { throw new UnsupportedOperationException("Read only"); }
}
