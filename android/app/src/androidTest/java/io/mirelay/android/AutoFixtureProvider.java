package io.mirelay.android;

import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.provider.DocumentsContract.Root;
import android.provider.DocumentsProvider;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.LinkedHashMap;
import java.util.Map;

/** Synthetic SAF provider in the separate test APK. No real filesystem access. */
public final class AutoFixtureProvider extends DocumentsProvider {
    static final String AUTHORITY = "io.mirelay.android.test.auto";
    private static final Map<String, Bundle> FILES = new LinkedHashMap<>();
    private static String listingMode = "";
    private static final String[] COLUMNS = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS};
    static synchronized Bundle control(Context context, String operation, Bundle args) {
        Uri tree = DocumentsContract.buildTreeDocumentUri(AUTHORITY, "root");
        switch (operation) {
            case "reset": FILES.clear(); listingMode = ""; break;
            case "put": FILES.put(args.getString("id"), new Bundle(args)); break;
            case "remove": FILES.remove(args.getString("id")); break;
            case "mode": listingMode = args.getString("mode", ""); break;
            case "grant": context.grantUriPermission("io.mirelay.android", tree,
                Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION); break;
            case "revoke": context.revokeUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION); break;
            default: throw new IllegalArgumentException("Unknown fixture operation");
        }
        Bundle result = new Bundle(); result.putString("tree", tree.toString()); return result;
    }
    @Override public boolean onCreate() { return true; }
    @Override public Cursor queryRoots(String[] projection) {
        String[] columns = projection == null ? new String[]{Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_FLAGS} : projection;
        MatrixCursor cursor = new MatrixCursor(columns);
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) {
            if (Root.COLUMN_ROOT_ID.equals(columns[i]) || Root.COLUMN_DOCUMENT_ID.equals(columns[i])) row[i] = "root";
            else if (Root.COLUMN_TITLE.equals(columns[i])) row[i] = "Auto fixtures";
            else if (Root.COLUMN_FLAGS.equals(columns[i])) row[i] = Root.FLAG_SUPPORTS_IS_CHILD;
        }
        cursor.addRow(row); return cursor;
    }
    @Override public synchronized Cursor queryDocument(String id, String[] projection) throws FileNotFoundException {
        synchronized (AutoFixtureProvider.class) {
            MatrixCursor cursor = new MatrixCursor(projection == null ? COLUMNS : projection);
            add(cursor, id, entry(id)); return cursor;
        }
    }
    @Override public Cursor queryChildDocuments(String parent, String[] projection, String sort) throws FileNotFoundException {
        synchronized (AutoFixtureProvider.class) {
            entry(parent);
            MatrixCursor cursor = new MatrixCursor(projection == null ? COLUMNS : projection);
            for (Map.Entry<String, Bundle> item : FILES.entrySet()) {
                if (parent.equals(item.getValue().getString("parent", "root"))) {
                    add(cursor, item.getKey(), item.getValue());
                    if ("duplicate".equals(listingMode)) add(cursor, item.getKey(), item.getValue());
                }
            }
            Bundle extras = new Bundle();
            if ("loading".equals(listingMode)) extras.putBoolean(DocumentsContract.EXTRA_LOADING, true);
            if ("error".equals(listingMode)) extras.putString(DocumentsContract.EXTRA_ERROR, "Synthetic incomplete listing");
            cursor.setExtras(extras); return cursor;
        }
    }
    private static Bundle entry(String id) throws FileNotFoundException {
        if ("root".equals(id)) {
            Bundle root = new Bundle(); root.putString("name", "Auto fixtures"); root.putBoolean("directory", true); return root;
        }
        Bundle entry = FILES.get(id);
        if (entry == null) throw new FileNotFoundException("Missing synthetic document");
        return entry;
    }
    private static void add(MatrixCursor cursor, String id, Bundle file) {
        Object[] row = new Object[cursor.getColumnCount()];
        String[] columns = cursor.getColumnNames();
        for (int i = 0; i < columns.length; i++) {
            if (Document.COLUMN_DOCUMENT_ID.equals(columns[i])) row[i] = id;
            else if (Document.COLUMN_DISPLAY_NAME.equals(columns[i])) row[i] = file.getString("name", id + ".bin");
            else if (Document.COLUMN_MIME_TYPE.equals(columns[i])) row[i] = file.getBoolean("directory") ? Document.MIME_TYPE_DIR : "application/octet-stream";
            else if (Document.COLUMN_SIZE.equals(columns[i])) row[i] = file.getBoolean("unknownSize") ? null : file.getLong("size", 4096);
            else if (Document.COLUMN_LAST_MODIFIED.equals(columns[i])) row[i] = file.getBoolean("unknownTime") ? null : file.getLong("modified", 1000);
            else if (Document.COLUMN_FLAGS.equals(columns[i])) row[i] = file.getBoolean("virtual") ? Document.FLAG_VIRTUAL_DOCUMENT : 0;
        }
        cursor.addRow(row);
    }
    @Override public boolean isChildDocument(String parent, String child) {
        synchronized (AutoFixtureProvider.class) {
            for (int depth = 0; depth < 20 && FILES.containsKey(child); depth++) {
                child = FILES.get(child).getString("parent", "root");
                if (parent.equals(child)) return true;
            }
            return false;
        }
    }
    @Override public ParcelFileDescriptor openDocument(String id, String mode, CancellationSignal signal) throws FileNotFoundException {
        final Bundle file;
        synchronized (AutoFixtureProvider.class) { file = new Bundle(entry(id)); }
        if (!"r".equals(mode)) throw new FileNotFoundException("Read only");
        long size = file.getLong("size", 4096);
        if (size < 0 || size > 101 * 1024 * 1024) throw new FileNotFoundException("Fixture too large");
        if (file.getBoolean("mutateOnRead")) synchronized (AutoFixtureProvider.class) {
            entry(id).putLong("modified", file.getLong("modified", 1000) + 1);
        }
        final ParcelFileDescriptor[] pipe;
        try { pipe = ParcelFileDescriptor.createReliablePipe(); }
        catch (IOException error) { throw new FileNotFoundException(error.getMessage()); }
        new Thread(() -> {
            try (ParcelFileDescriptor.AutoCloseOutputStream output = new ParcelFileDescriptor.AutoCloseOutputStream(pipe[1])) {
                byte[] buffer = new byte[65536]; int written = 0;
                byte[] exact = file.getByteArray("bytes");
                while (written < size) {
                    if (signal != null && signal.isCanceled()) break;
                    if (file.getInt("delayMillis", 0) > 0) Thread.sleep(Math.min(500, file.getInt("delayMillis")));
                    int count = (int)Math.min(buffer.length, size - written);
                    if (exact != null) {
                        if (exact.length != size) throw new IOException("Incorrect fixture length");
                        System.arraycopy(exact, written, buffer, 0, count);
                    } else for (int i = 0; i < count; i++) buffer[i] = (byte)((written + i + file.getInt("contentOffset", 0)) % 251);
                    output.write(buffer, 0, count); written += count;
                }
            } catch (IOException ignored) { }
            catch (InterruptedException ignored) { Thread.currentThread().interrupt(); }
        }, "auto-fixture").start();
        return pipe[0];
    }
}
