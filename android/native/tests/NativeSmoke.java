import io.mirelay.android.NativeBridge;
import java.nio.file.Files;
import java.nio.file.Path;

public final class NativeSmoke {
    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    public static void main(String[] args) throws Exception {
        check(NativeBridge.initialize(null), "host initialization failed");
        check(NativeBridge.upload("not JSON", (a, b) -> true).contains("Invalid upload configuration"), "invalid JSON guard");
        check(NativeBridge.upload("x".repeat(65537), (a, b) -> true).contains("too large"), "large request guard");
        if (args.length == 0) { System.out.println("JNI input checks passed"); return; }
        String request = Files.readString(Path.of(args[0]));
        boolean propagated = false;
        try { NativeBridge.upload(request, (a, b) -> { throw new IllegalStateException("callback-test"); }); }
        catch (IllegalStateException expected) { propagated = expected.getMessage().equals("callback-test"); }
        check(propagated, "callback exception did not propagate safely");
        String paused = NativeBridge.upload(request, (a, b) -> a < 3);
        check(paused.contains("\"uploaded_bytes\":3") && paused.contains("\"delivery_id\":null"), "pause failed: " + paused);
        String complete = NativeBridge.upload(request, (a, b) -> true);
        check(!complete.contains("\"error\"") && !complete.contains("\"delivery_id\":null"), "upload failed: " + complete);
        String replay = NativeBridge.upload(request, (a, b) -> true);
        check(replay.equals(complete), "completed session replay differs");
        System.out.println("JNI upload, pause, resume, receipt replay, and callback exception checks passed");
    }
}
