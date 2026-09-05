package io.mirelay.android;

/** Shared Rust sender. Call only from a worker thread; no Android dependencies. */
public final class NativeBridge {
    static { System.loadLibrary("mirelay_android"); }
    private NativeBridge() {}
    public static native boolean initialize(Object applicationContext);
    public interface Progress {
        /** Return false to pause at a request boundary. */
        boolean update(long uploaded, long total);
    }
    public static native String upload(String requestJson, Progress progress);
    public static native String pairing(String requestJson);
    public static native String directory(String requestJson);
}
