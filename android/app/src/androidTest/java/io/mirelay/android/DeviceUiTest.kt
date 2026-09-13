package io.mirelay.android

import android.content.ClipData
import android.content.Intent
import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import androidx.work.WorkManager
import kotlinx.coroutines.async
import org.junit.Assert.*
import org.junit.Before
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.TimeUnit
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class DeviceUiTest {
    @get:Rule val ui = createEmptyComposeRule()
    private lateinit var activity: MainActivity
    private val app get() = DeviceSupport.app
    private val device get() = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
    private fun model() = ViewModelProvider(activity)[RelayViewModel::class.java]
    // ActivityScenario matches the launch Intent, and cannot track a singleTop
    // activity after setIntent(SEND). Observe real lifecycle events instead.
    private val lifecycle = object : Application.ActivityLifecycleCallbacks {
        override fun onActivityCreated(a: Activity, state: Bundle?) { if (a is MainActivity) activity = a }
        override fun onActivityStarted(a: Activity) { }
        override fun onActivityResumed(a: Activity) { }
        override fun onActivityPaused(a: Activity) { }
        override fun onActivityStopped(a: Activity) { }
        override fun onActivitySaveInstanceState(a: Activity, state: Bundle) { }
        override fun onActivityDestroyed(a: Activity) { }
    }
    @Before fun setup() {
        DeviceSupport.guard()
        DeviceSupport.reset()
        app.getSharedPreferences("appearance", 0).edit().clear().commit()
        app.registerActivityLifecycleCallbacks(lifecycle)
        InstrumentationRegistry.getInstrumentation().startActivitySync(Intent(app, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
        ui.waitUntil(15000) { !model().busy.value }
        ui.runOnIdle { model().pending.value = emptyList(); model().selected.value = null; model().error.value = null }
    }
    @After fun teardown() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync { if (::activity.isInitialized) activity.finish() }
        app.unregisterActivityLifecycleCallbacks(lifecycle)
    }
    private fun select(folder: String) { ui.runOnIdle { model().selected.value = folder }; ui.waitForIdle() }

    @Test fun disconnectRequiresConfirmationAndRemovalKeepsStagedFiles() {
        val invite = DeviceSupport.invitation()
        val connection = FolderConnection(app)
        val secret = connection.newSenderToken()
        val claimed = connection.claim(DeviceSupport.server, invite.getString("pairing_code"), secret, true)
        val folder = app.store.saveFolder(null, "Disconnect QA", connection.folderUrl(DeviceSupport.server, invite.getString("pairing_code")), secret, true, "awaiting_confirmation")
        app.store.pairingResult(folder, "awaiting_confirmation", claimed.getString("verification"))
        val transfer = DeviceSupport.import(folder, name = "keep-private.bin")
        val staged = File(app.store.directory(transfer), "payload")
        val bytes = staged.readBytes()
        select(folder)
        ui.onNodeWithContentDescription("Folder settings").performClick()
        ui.onNodeWithTag("remove-folder").assertIsNotEnabled()
        ui.onNodeWithTag("disconnect-folder").performClick()
        ui.onNodeWithText("Disconnect Folder?").assertIsDisplayed()
        // Cancelling never changes durable state.
        ui.onAllNodesWithText("Cancel").onLast().performClick()
        assertEquals("awaiting_confirmation", app.store.folder(folder)!!.pairingState)
        ui.onNodeWithTag("disconnect-folder").performClick()
        ui.onAllNodesWithText("Disconnect").onLast().performClick()
        ui.waitUntil(20000) { !model().busy.value && app.store.folder(folder)?.pairingState == "disconnected" }
        assertFalse(app.store.connectionOpen(folder))
        assertEquals(TransferStatus.PAUSED, app.store.transfer(transfer)!!.status)
        assertTrue(runCatching { app.uploads.enqueue(transfer, manual = true) }.isFailure)
        assertTrue(runCatching { app.store.saveFolder(folder, "Changed", app.store.folder(folder)!!.server, secret, true, "ready") }.isFailure)
        assertArrayEquals(bytes, staged.readBytes())
        ui.onNodeWithText("Save").assertIsNotEnabled()
        ui.onNodeWithTag("remove-folder").assertIsEnabled().performClick()
        ui.onNodeWithText("Remove Folder?").assertIsDisplayed()
        ui.onNodeWithText("Remove", substring = false).performClick()
        ui.waitUntil(15000) { !model().busy.value && app.store.folder(folder) == null }
        assertNull(app.store.transfer(transfer))
        assertArrayEquals(bytes, staged.readBytes())
        assertEquals("disconnected", DeviceSupport.setupRequest("/f/${invite.getString("folder_id")}/api/v1/pairing/disconnect", invite.getString("receiver_token"), org.json.JSONObject()).getString("state"))
    }

    @Test fun failedDisconnectKeepsCredentialAndDurableBarrierAgainstStaleWork() {
        val folder = DeviceSupport.folder("Offline pairing", "http://127.0.0.1:1/f/${UUID.randomUUID()}")
        val other = DeviceSupport.folder("Unrelated")
        val transfer = DeviceSupport.import(folder)
        val work = UUID.randomUUID().toString()
        app.store.assign(transfer, work)
        ui.runOnIdle { model().disconnectFolder(folder) }
        ui.waitUntil(20000) { !model().busy.value && app.store.folder(folder)?.pairingState == "disconnect_pending" }
        assertEquals(DeviceSupport.token, app.store.token(folder))
        assertFalse(app.store.updateOwned(transfer, work, TransferStatus.UPLOADED, delivery = "stale"))
        app.store.pairingResult(folder, "ready", null)
        assertEquals("disconnect_pending", app.store.folder(folder)!!.pairingState)
        assertTrue(runCatching { app.store.removeFolder(folder) }.isFailure)
        assertTrue(runCatching { app.store.automatic.enable(folder, "content://unused/tree/source", "Source", true, emptyList(), 0) }.isFailure)
        assertTrue(runCatching { DeviceSupport.import(folder) }.isFailure)
        app.uploads.recover()
        assertEquals(TransferStatus.PAUSED, app.store.transfer(transfer)!!.status)
        assertTrue(app.store.connectionOpen(other))
        select(folder)
        ui.onNodeWithContentDescription("Folder settings").performClick()
        ui.onNodeWithText("Retry disconnect").assertIsDisplayed()
        ui.onNodeWithTag("remove-folder").assertIsNotEnabled()
    }

    @Test fun legacyRemovalOnlyDeletesLocalRecordsAndRetainsFiles() {
        val folder = DeviceSupport.folder()
        val other = DeviceSupport.folder("Other")
        val transfer = DeviceSupport.import(folder)
        val staged = File(app.store.directory(transfer), "payload")
        val bytes = staged.readBytes()
        ui.runOnIdle { model().removeFolder(folder) {} }
        ui.waitUntil(15000) { !model().busy.value && app.store.folder(folder) == null }
        assertArrayEquals(bytes, staged.readBytes())
        assertTrue(app.store.connectionOpen(other))
        assertNull(app.store.transfer(transfer))
    }
    private fun share(multiple: Boolean = false) {
        val uri = DeviceSupport.uri(8192, "Shared fixture.bin")
        val intent = Intent(if (multiple) Intent.ACTION_SEND_MULTIPLE else Intent.ACTION_SEND).apply {
            setClass(app, MainActivity::class.java); type = "application/octet-stream"
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_GRANT_READ_URI_PERMISSION)
            clipData = ClipData.newRawUri("fixture", uri)
            if (multiple) putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(uri, DeviceSupport.uri(17, "Second.bin")))
            else putExtra(Intent.EXTRA_STREAM, uri)
        }
        app.startActivity(intent)
    }
    @Test fun emptyStateOpensFolderEditorAndGuardsBlankInput() {
        assertTrue(app.packageManager.getApplicationIcon(app.packageName) is android.graphics.drawable.AdaptiveIconDrawable)
        assertTopLeftWordmark()
        ui.onNodeWithText("Your files.\nYour server.").assertIsDisplayed()
        saveScreen("brand-welcome")
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        ui.onNodeWithText("Save").assertIsNotEnabled()
        ui.onNodeWithText("Cancel").performClick()
    }
    private fun scanResult(contents: String?, denied: Boolean = false) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(app.packageName, android.Manifest.permission.CAMERA)
        val result = Intent().apply {
            if (contents != null) {
                putExtra(com.google.zxing.client.android.Intents.Scan.RESULT, contents)
                putExtra(com.google.zxing.client.android.Intents.Scan.RESULT_FORMAT, "QR_CODE")
            }
            if (denied) putExtra(com.google.zxing.client.android.Intents.Scan.MISSING_CAMERA_PERMISSION, true)
        }
        val monitor = instrumentation.addMonitor(InvitationScannerActivity::class.java.name,
            android.app.Instrumentation.ActivityResult(if (contents == null) Activity.RESULT_CANCELED else Activity.RESULT_OK, result), true)
        try {
            ui.onNodeWithTag("scan-invitation").performScrollTo().performClick()
            ui.waitForIdle()
            assertEquals(1, monitor.hits)
        } finally { instrumentation.removeMonitor(monitor) }
    }
    @Test fun qrScanFillsInvitationWithoutSavingOrEnablingAuto() {
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        val code = "00000000-0000-4000-8000-000000000001." + "ab".repeat(32)
        val text = org.json.JSONObject().put("kind", "mirelay-pairing").put("version", 1)
            .put("server_url", "https://example.com").put("pairing_code", code)
            .put("expires_at_unix", System.currentTimeMillis()/1000+600).toString()
        scanResult(text)
        ui.onNodeWithText("Server URL").assertTextContains("https://example.com")
        ui.onNodeWithTag("pair-folder").assertIsOn()
        ui.onNodeWithTag("setup-http").performScrollTo().assertIsOff()
        assertTrue(app.store.folders.value.isEmpty())
        assertTrue(app.store.automatic.sources.value.isEmpty())
        ui.onNodeWithText("Cancel").performClick()
        assertTrue(app.store.folders.value.isEmpty())
    }
    @Test fun qrInvalidResultAndCancellationPreserveManualInput() {
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        ui.onNodeWithText("Server URL").performTextInput("https://manual.example.com")
        scanResult("https://not-a-mirelay-invitation.example")
        ui.onNodeWithTag("scan-error").performScrollTo().assertTextContains("Not a valid MiRelay invitation", substring=true)
        ui.onNodeWithText("Server URL").assertTextContains("https://manual.example.com")
        scanResult(null)
        ui.onNodeWithTag("scan-error").assertDoesNotExist()
        ui.onNodeWithText("Server URL").assertTextContains("https://manual.example.com")
        assertTrue(app.store.folders.value.isEmpty())
    }
    @Test fun qrMissingPermissionResultKeepsManualPairingAvailable() {
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        scanResult(null, denied=true)
        ui.onNodeWithTag("scan-error").performScrollTo().assertTextContains("Camera permission is required", substring=true)
        ui.onNodeWithText("Server URL").performScrollTo().performTextInput("https://manual.example.com")
        assertTrue(app.store.folders.value.isEmpty())
    }
    @Test fun folderHeaderUsesOriginalWordmarkAndMenuStillOpensDrawer() {
        select(DeviceSupport.folder())
        assertTopLeftWordmark()
        ui.onNodeWithText("Auto").assertIsDisplayed().assertIsEnabled()
        ui.onNodeWithContentDescription("Folder settings").assertIsDisplayed().assertIsEnabled()
        saveScreen("brand-folder-header")
        ui.onNodeWithContentDescription("Folders").performClick()
        ui.onNodeWithTag("drawer-brand-wordmark").assertIsDisplayed()
        ui.onNodeWithText("MiRelay").assertDoesNotExist()
        saveScreen("brand-folder-drawer")
    }
    @Test fun selectedFolderIconsHaveWhiteInkInBothThemes() {
        val folder = DeviceSupport.folder()
        val night = device.executeShellCommand("cmd uimode night").trim().substringAfterLast(' ')
        try {
            for (dark in listOf(false, true)) {
                device.executeShellCommand("cmd uimode night ${if (dark) "yes" else "no"}")
                ui.waitUntil(15000) {
                    activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
                        (if (dark) Configuration.UI_MODE_NIGHT_YES else Configuration.UI_MODE_NIGHT_NO) && !model().busy.value
                }
                select(folder)
                // Theme recreation can restore an already-open drawer. Do not
                // toggle it closed, and wait for its visibility before sampling.
                val icon = ui.onNodeWithTag("folder-icon-$folder", useUnmergedTree = true)
                if (!icon.isDisplayed()) ui.onNodeWithContentDescription("Folders").performClick()
                ui.waitUntil(15000) { icon.isDisplayed() }
                for (kind in FolderKind.entries) {
                    app.store.setFolderKind(folder, kind); ui.waitForIdle()
                    android.os.SystemClock.sleep(350)
                    val bounds = ui.onNodeWithTag("folder-icon-$folder", useUnmergedTree = true).assertIsDisplayed().fetchSemanticsNode().boundsInWindow
                    val bitmap = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
                    try {
                        var white = 0; var black = 0
                        for (y in bounds.top.toInt() until bounds.bottom.toInt()) for (x in bounds.left.toInt() until bounds.right.toInt()) {
                            val pixel = bitmap.getPixel(x, y)
                            if (Color.red(pixel) > 230 && Color.green(pixel) > 230 && Color.blue(pixel) > 230) white++
                            if (Color.red(pixel) < 40 && Color.green(pixel) < 40 && Color.blue(pixel) < 40) black++
                        }
                        assertTrue("Selected $kind icon needs white strokes on black: $white / $black", white > 20 && black > white)
                        writeScreen(bitmap, "selected-icon-${kind.key}-$dark")
                    } finally { bitmap.recycle() }
                }
            }
        } finally { device.executeShellCommand("cmd uimode night $night") }
    }
    @Test fun photosRenderOriginalLocalArtAndCategorySwitchDoesNotChangeTransfers() {
        val folder=DeviceSupport.folder()
        val id=UUID.randomUUID().toString()
        val dir=app.store.directory(id);assertTrue(dir.mkdirs())
        val payload=File(dir,"payload")
        app.resources.openRawResource(R.drawable.mirelay_brand_icon).use {input -> payload.outputStream().use {input.copyTo(it)} }
        assertNotNull(PhotoThumbnails.decode(payload,256))
        app.store.addTransfer(id,folder,"Original.png",payload.length())
        app.store.assign(id,"fixture-owner")
        app.store.updateOwned(id,"fixture-owner",TransferStatus.UPLOADED,payload.length())
        val before=app.store.transfer(id)
        select(folder)
        ui.onNodeWithTag("folder-kind").performClick()
        ui.onNodeWithTag("kind-photos").performClick()
        ui.waitUntil(15000) {!model().busy.value && app.store.folders.value.single().kind==FolderKind.PHOTOS}
        ui.onNodeWithTag("album-grid").performScrollToNode(hasTestTag("photo-card-$id"))
        // The clickable tile merges its image semantics into the parent.
        ui.waitUntil(15000) {ui.onAllNodesWithTag("photo-image-$id",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithTag("photo-image-$id",useUnmergedTree=true).assertIsDisplayed().performTouchInput {click()}
        ui.onNodeWithTag("album-viewer").assertIsDisplayed()
        // Platform dialog-window fades are not part of Compose's idle clock.
        android.os.SystemClock.sleep(350)
        saveScreen("photos-preview")
        ui.onNodeWithContentDescription("Close photo").performClick()
        ui.onNodeWithTag("album-viewer").assertDoesNotExist()
        android.os.SystemClock.sleep(350)
        saveScreen("photos-grid")
        ui.onNodeWithContentDescription("Album options").performClick()
        ui.onNodeWithText("General view").performClick()
        ui.waitUntil(15000) {!model().busy.value && app.store.folders.value.single().kind==FolderKind.GENERAL}
        ui.onNodeWithTag("photo-card-$id").assertDoesNotExist()
        assertEquals(before,app.store.transfer(id));assertTrue(payload.isFile)
    }
    @Test fun realUploadCleanupKeepsPreviewAcrossActivityRecreationAndFreshDiskReader() {
        val folder = DeviceSupport.folder()
        app.store.setFolderKind(folder, FolderKind.PHOTOS)
        val id = UUID.randomUUID().toString()
        val dir = app.store.directory(id).apply { mkdirs() }
        val payload = File(dir, "payload")
        app.resources.openRawResource(R.drawable.mirelay_brand_icon).use { input -> payload.outputStream().use { input.copyTo(it) } }
        app.store.addTransfer(id, folder, "Uploaded-original.png", payload.length())
        val corrupt = DeviceSupport.import(folder, name = "Uploaded-invalid.jpg")
        for (transfer in listOf(id, corrupt)) app.uploads.enqueue(transfer, manual = true)
        DeviceSupport.await(60000) {
            listOf(id, corrupt).all { app.store.transfer(it)?.status == TransferStatus.UPLOADED && !File(app.store.directory(it), "payload").exists() }
        }
        assertFalse(File(dir, "resume.json").exists())
        // No in-memory UI cache has seen this transfer. A fresh reader can only
        // obtain the independently persisted, derived preview after cleanup.
        val disk = PhotoPreviewCache(File(app.cacheDir, "photo-previews"))
        assertNotNull(disk.load(id, 256)); assertNotNull(disk.load(id, 1024))
        assertNull(disk.load(corrupt, 256))
        assertNotNull(app.store.transfer(corrupt)!!.deliveryId)
        select(folder)
        InstrumentationRegistry.getInstrumentation().runOnMainSync { activity.recreate() }
        ui.waitUntil(15000) { !model().busy.value }
        select(folder)
        ui.onNodeWithTag("album-grid").performScrollToNode(hasTestTag("photo-card-$id"))
        ui.waitUntil(15000) { ui.onAllNodesWithTag("photo-image-$id", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        ui.onNodeWithTag("photo-image-$id", useUnmergedTree = true).assertIsDisplayed().performTouchInput { click() }
        ui.onNodeWithTag("album-viewer").assertIsDisplayed()
        ui.waitUntil(15000) { ui.onAllNodes(hasTestTag("photo-image-$id") and hasAnyAncestor(isDialog()), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        ui.onNode(hasTestTag("photo-image-$id") and hasAnyAncestor(isDialog()), useUnmergedTree = true).assertIsDisplayed()
        android.os.SystemClock.sleep(350)
        saveScreen("photos-real-upload-preview")
        ui.onNodeWithContentDescription("Close photo").performClick()
    }
    @Test fun corruptImageFallsBackAndPendingTransfersRemainInActivityPanel() {
        val folder=DeviceSupport.folder();app.store.setFolderKind(folder,FolderKind.PHOTOS)
        val id=DeviceSupport.import(folder,name="broken.jpg")
        app.store.assign(id,"fixture-owner");app.store.updateOwned(id,"fixture-owner",TransferStatus.UPLOADED,4096)
        val pending=DeviceSupport.import(folder,name="pending.jpg");app.store.pause(pending)
        assertNull(PhotoThumbnails.decode(File(app.store.directory(id),"payload"),256))
        select(folder)
        ui.onNodeWithTag("album-grid").performScrollToNode(hasTestTag("photo-card-$id"))
        ui.onNodeWithContentDescription("Preview unavailable: broken.jpg", useUnmergedTree = true).assertIsDisplayed()
        ui.onNodeWithTag("album-status").performClick()
        ui.onNodeWithText("pending.jpg").assertIsDisplayed()
        ui.onNodeWithContentDescription("Resume or retry transfer").assertIsDisplayed()
        ui.onNodeWithText("Close").performClick()
    }
    private fun sourceAlbum(count: Int = 6): String {
        AutoFixture.reset()
        val bytes = app.resources.openRawResource(R.drawable.mirelay_brand_icon).use { it.readBytes() }
        repeat(count) { index -> AutoFixture.put("album-$index", bytes.size.toLong(), extra = {
            putString("name", "Image-$index.png"); putByteArray("bytes", bytes); putLong("modified", 1000L + index)
        }) }
        AutoFixture.put("notes", 20, extra = { putString("name", "notes.txt") })
        val folder = DeviceSupport.folder("Album QA")
        app.auto.enable(folder, AutoFixture.tree, true, startImmediately = false)
        app.store.setFolderKind(folder, FolderKind.PHOTOS)
        select(folder)
        ui.waitUntil(15000) { ui.onAllNodesWithTag("photo-card-album-${count - 1}").fetchSemanticsNodes().isNotEmpty() }
        return folder
    }
    @Test fun sourceAlbumShowsExistingFilesWithoutUploadingAndSupportsFullscreenZoomAndPaging() {
        try {
            val folder = sourceAlbum()
            val before = app.store.automatic.source(folder)
            ui.onNodeWithText("Transfers").assertDoesNotExist()
            ui.onNodeWithText("Image-5.png").assertDoesNotExist() // No filename under the grid tile.
            ui.onNodeWithTag("photo-card-album-5").performClick()
            ui.onNodeWithTag("album-viewer").assertIsDisplayed()
            ui.waitUntil(15000) { ui.onAllNodes(hasTestTag("photo-image-album-5") and hasAnyAncestor(isDialog()), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
            val zoom = ui.onNodeWithTag("album-zoom-album-5")
            zoom.performTouchInput { doubleClick(center) }
            zoom.assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.StateDescription, "Zoomed"))
            zoom.performTouchInput { doubleClick(center) }
            zoom.assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.StateDescription, "Fit"))
            ui.onNodeWithTag("album-pager").performTouchInput { swipeLeft() }
            ui.waitUntil(10000) { ui.onAllNodesWithText("2 / 6").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithContentDescription("Photo details").performClick()
            ui.onNodeWithText("Optimized preview · original file unchanged").assertIsDisplayed()
            ui.onNodeWithText("Close").performClick()
            android.os.SystemClock.sleep(350); saveScreen("album-fullscreen")
            ui.onNodeWithContentDescription("Close photo").performClick()
            assertEquals(before, app.store.automatic.source(folder))
            assertTrue(app.store.transfers.value.isEmpty())
            assertTrue(File(app.cacheDir, "album-scratch").listFiles().orEmpty().isEmpty())
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun albumGlassScrollsOverImagesAndReducedTransparencyPersists() {
        try {
            val folder = sourceAlbum(36)
            ui.waitUntil(15000) { ui.onAllNodesWithTag("photo-image-album-35", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithTag("album-glass").assertIsDisplayed()
            ui.onNodeWithTag("album-grid").performTouchInput { swipeUp() }
            ui.waitForIdle(); android.os.SystemClock.sleep(500); saveScreen("album-frosted-scroll")
            ui.onNodeWithTag("album-title").assertIsDisplayed()
            ui.onNodeWithContentDescription("Album options").performClick()
            ui.onNodeWithText("Reduce transparency").performClick()
            device.pressBack(); ui.waitForIdle()
            ui.onNodeWithTag("album-glass-fallback").assertIsDisplayed()
            assertTrue(app.getSharedPreferences("appearance", 0).getBoolean("reduce_transparency", false))
            saveScreen("album-reduced-transparency")
            InstrumentationRegistry.getInstrumentation().runOnMainSync { activity.recreate() }
            ui.waitUntil(15000) { !model().busy.value }; select(folder)
            ui.onNodeWithTag("album-glass-fallback").assertIsDisplayed()
            assertTrue(app.store.transfers.value.isEmpty())
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun albumRefreshRemovesDeletedFilesAndRevokedPermissionNeverShowsPartialLibrary() {
        try {
            sourceAlbum()
            AutoFixture.control("remove", android.os.Bundle().apply { putString("id", "album-5") })
            ui.onNodeWithContentDescription("Refresh photos").performClick()
            ui.waitUntil(15000) { ui.onAllNodesWithTag("photo-card-album-5").fetchSemanticsNodes().isEmpty() && ui.onAllNodesWithTag("photo-card-album-4").fetchSemanticsNodes().isNotEmpty() }
            AutoFixture.mode("loading")
            ui.onNodeWithContentDescription("Refresh photos").performClick()
            ui.waitUntil(15000) { ui.onAllNodesWithText("Could not read this Folder completely.", substring = true).fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithTag("photo-card-album-4").assertDoesNotExist()
            AutoFixture.mode(""); AutoFixture.control("revoke")
            ui.onNodeWithText("Try again").performClick()
            ui.waitUntil(15000) { ui.onAllNodesWithText("Folder access is unavailable.", substring = true).fetchSemanticsNodes().isNotEmpty() }
            assertTrue(app.store.transfers.value.isEmpty())
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun albumReaderBoundsFilesRejectsChangedMetadataAndCancelsBlockedReads() {
        try {
            AutoFixture.reset()
            app.contentResolver.takePersistableUriPermission(AutoFixture.tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            AutoFixture.put("large", AlbumLibrary.MAX_PREVIEW_BYTES + 1, extra = { putString("name", "large.jpg") })
            AutoFixture.put("changing", 1024, extra = { putString("name", "changing.jpg"); putBoolean("mutateOnRead", true) })
            AutoFixture.put("slow", 1024 * 1024, extra = { putString("name", "slow.jpg"); putInt("delayMillis", 500) })
            kotlinx.coroutines.runBlocking {
                val photos = app.album.scan(AutoFixture.tree.toString()).associateBy { it.key }
                assertNull(app.album.load(photos.getValue("large"), app.store, 256))
                var changed = false
                try { app.album.load(photos.getValue("changing"), app.store, 256) } catch (_: IllegalStateException) { changed = true }
                assertTrue(changed)
                repeat(5) {
                    val start = android.os.SystemClock.elapsedRealtime()
                    try { kotlinx.coroutines.withTimeout(150) { app.album.load(photos.getValue("slow"), app.store, 256) }; fail("Read must cancel") }
                    catch (_: kotlinx.coroutines.TimeoutCancellationException) { }
                    assertTrue("Cancellation must not wait for the 15-second provider deadline", android.os.SystemClock.elapsedRealtime() - start < 3000)
                }
            }
            assertTrue(File(app.cacheDir, "album-scratch").listFiles().orEmpty().isEmpty())
            assertTrue(app.store.transfers.value.isEmpty())
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun albumBackgroundCancelsReadsAndForegroundRefreshesWithoutUploads() {
        try {
            AutoFixture.reset()
            repeat(20) { index -> AutoFixture.put("slow-$index", 1024 * 1024, extra = {
                putString("name", "Slow-$index.jpg"); putInt("delayMillis", 500)
            }) }
            val folder = DeviceSupport.folder("Background album QA")
            app.auto.enable(folder, AutoFixture.tree, true, startImmediately = false)
            app.store.setFolderKind(folder, FolderKind.PHOTOS)
            select(folder)
            DeviceSupport.await(10000) { AutoFixture.opens() > 0 }
            device.pressHome()
            DeviceSupport.await(5000) {
                !activity.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED) &&
                    File(app.cacheDir, "album-scratch").listFiles().orEmpty().isEmpty()
            }
            val opens = AutoFixture.opens()
            android.os.SystemClock.sleep(1000)
            assertEquals("Hidden album must not open more sources", opens, AutoFixture.opens())
            repeat(20) { index -> AutoFixture.control("remove", Bundle().apply { putString("id", "slow-$index") }) }
            val bytes = app.resources.openRawResource(R.drawable.mirelay_brand_icon).use { it.readBytes() }
            AutoFixture.put("resumed", bytes.size.toLong(), extra = { putString("name", "Resumed.png"); putByteArray("bytes", bytes) })
            app.startActivity(Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
            ui.waitUntil(15000) { ui.onAllNodesWithTag("photo-image-resumed", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithTag("photo-card-slow-0").assertDoesNotExist()
            assertTrue(app.store.transfers.value.isEmpty())
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun albumConcurrentRequestsReadSourceOnceAndRefreshInvalidatesCache() {
        try {
            AutoFixture.reset()
            app.contentResolver.takePersistableUriPermission(AutoFixture.tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            val bytes = app.resources.openRawResource(R.drawable.mirelay_brand_icon).use { it.readBytes() }
            AutoFixture.put("shared", bytes.size.toLong(), extra = { putString("name", "Shared.png"); putByteArray("bytes", bytes) })
            kotlinx.coroutines.runBlocking {
                val photo = app.album.scan(AutoFixture.tree.toString()).single()
                val results = kotlinx.coroutines.coroutineScope {
                    List(20) { async { app.album.load(photo, app.store, 256) } }.map { it.await() }
                }
                assertNotNull(results.first()); assertTrue(results.all { it === results.first() })
                assertEquals(1, AutoFixture.opens())
                val refreshed = app.album.scan(AutoFixture.tree.toString()).single()
                assertNotNull(app.album.load(refreshed, app.store, 256))
                assertEquals(2, AutoFixture.opens())
                AutoFixture.control("revoke")
                try { app.album.load(refreshed, app.store, 256); fail("Cached image must require permission") }
                catch (_: SecurityException) { }
            }
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun drawerSurvivesRepeatedEmptyAndRepopulatedLists() {
        repeat(12) { index ->
            val id=DeviceSupport.folder("Transient $index")
            select(id)
            ui.onNodeWithContentDescription("Folders").performClick()
            ui.onNodeWithTag("folder-drawer").performScrollToIndex(2)
            // Exercise the visible drawer shrinking to only heading + Add Folder.
            app.store.writableDatabase.execSQL("DELETE FROM folders")
            app.store.refresh()
            ui.waitForIdle()
            ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("folder-drawer"))).assertIsDisplayed()
            val replacement=DeviceSupport.folder("Replacement $index")
            ui.onNode(hasText("Replacement $index") and hasAnyAncestor(hasTestTag("folder-drawer"))).assertIsDisplayed().performClick()
            ui.waitForIdle()
            assertEquals(replacement,model().selected.value)
            // Also change the off-screen drawer while the main content is alive.
            app.store.writableDatabase.execSQL("DELETE FROM folders");app.store.refresh()
            ui.waitForIdle();ui.onNodeWithTag("welcome").assertIsDisplayed()
        }
    }
    private fun assertTopLeftWordmark() {
        val mark = ui.onNodeWithTag("brand-wordmark").assertIsDisplayed().fetchSemanticsNode().boundsInRoot
        val bar = ui.onNodeWithTag("brand-top-bar").fetchSemanticsNode().boundsInRoot
        assertTrue("Wordmark must stay at the left edge", mark.left - bar.left < bar.width * 0.1f)
        assertTrue("Wordmark must fit inside the app bar", mark.top >= bar.top && mark.bottom <= bar.bottom)
        assertTrue("Wordmark must stay left of the toolbar actions", mark.right <=
            ui.onNodeWithContentDescription("Folders").assertIsDisplayed().fetchSemanticsNode().boundsInRoot.left)
        // ModalDrawer keeps its off-screen content composed while closed.
        // Check the header and welcome separately, not all composed nodes.
        val branded = hasContentDescription("MiRelay — Send · Receive — by MiiiKuuu")
        ui.onAllNodes(branded and hasAnyAncestor(hasTestTag("brand-top-bar"))).assertCountEquals(1)
        ui.onAllNodes(branded and hasAnyAncestor(hasTestTag("welcome"))).assertCountEquals(0)
        ui.onNodeWithTag("drawer-brand-wordmark").assertIsNotDisplayed()
        ui.onNodeWithText("MiRelay").assertDoesNotExist()
    }
    @Test fun autoDirectoryPickerCanBeCancelledWithoutConsentOrTransfer() {
        select(DeviceSupport.folder())
        ui.onNodeWithText("Auto").performClick()
        ui.onNodeWithText("Enable Auto").assertIsNotEnabled()
        ui.onNodeWithText("Choose source directory").performScrollTo().performClick()
        DeviceSupport.await(10000) { device.currentPackageName != app.packageName }
        device.pressBack()
        DeviceSupport.await(10000) { device.currentPackageName == app.packageName }
        ui.onNodeWithText("Enable Auto").assertIsNotEnabled()
        ui.onNodeWithText("Close").performClick()
        assertTrue(app.store.automatic.sources.value.isEmpty()); assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun autoRequiresExplicitEnableAndCanBePausedThroughUi() {
        AutoFixture.reset()
        val folder = DeviceSupport.folder()
        try {
            AutoFixture.put("historical")
            select(folder); ui.onNodeWithText("Auto").performClick()
            // A picker result only fills the draft; it cannot start sending.
            ui.runOnIdle { model().acceptAutoTree(folder, AutoFixture.tree) }
            assertTrue(app.store.automatic.sources.value.isEmpty())
            ui.onNodeWithText("Enable Auto").assertIsEnabled()
            InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("pm grant ${app.packageName} android.permission.POST_NOTIFICATIONS").close()
            ui.onNodeWithText("Enable Auto").performClick()
            ui.waitUntil(15000) { app.store.automatic.sources.value.singleOrNull()?.enabled == true && !model().busy.value }
            saveScreen("auto-enabled")
            ui.onNodeWithText("Pause Auto").assertIsDisplayed().performClick()
            ui.waitUntil(15000) { app.store.automatic.sources.value.singleOrNull()?.enabled == false && !model().busy.value }
            ui.onNodeWithText("Enable Auto").assertIsDisplayed()
            ui.onNodeWithText("Close").performClick()
            assertTrue(app.store.transfers.value.isEmpty())
        } finally { app.auto.pause(folder); AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun autoDialogControlsRemainReachableInShortWindowWithLargeText() {
        select(DeviceSupport.folder())
        withDisplay(size = "720x640", fontScale = 1.5f) {
            ui.onNodeWithText("Auto").performClick()
            ui.onNodeWithText("Enable Auto").assertIsDisplayed().assertIsNotEnabled()
            ui.onNodeWithText("Choose source directory").performScrollTo().assertIsDisplayed()
            saveScreen("auto-short-window")
            ui.onNodeWithText("Close").assertIsDisplayed().performClick()
        }
    }
    @Test fun createAndRenameFolderThroughRealUi() {
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        ui.onNodeWithText("Name").performTextInput("Desktop")
        ui.onNodeWithText("Server URL").performTextInput(DeviceSupport.server)
        ui.onNodeWithTag("pair-folder").performScrollTo().performClick()
        ui.onNodeWithText("Bearer token").performScrollTo().performTextInput(DeviceSupport.token)
        ui.onNodeWithTag("setup-http").performScrollTo().performClick()
        ui.onNodeWithText("Save").performClick()
        ui.waitUntil(15000) { app.store.folders.value.size == 1 }
        ui.onNodeWithContentDescription("Folder settings").performClick()
        ui.onNodeWithText("Server URL").assertIsNotEnabled()
        ui.onNodeWithText("Name").performTextReplacement("Renamed")
        ui.onNodeWithText("Save").performClick()
        ui.waitUntil(15000) { app.store.folders.value.firstOrNull()?.name == "Renamed" }
        assertEquals(DeviceSupport.token, app.store.token(app.store.folders.value.single().id))
    }
    @Test fun newFolderSelectsExistingDirectoryAndWaitsForLinuxConfirmation() {
        AutoFixture.reset()
        try {
            AutoFixture.put("historical")
            val invite = DeviceSupport.invitation()
            ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
            ui.onNodeWithText("Choose existing directory").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("Name").performScrollTo().performTextInput("Pixiv")
            ui.onNodeWithText("Server URL").performScrollTo().performTextInput(DeviceSupport.server)
            ui.runOnIdle { model().acceptAutoTree("new-folder", AutoFixture.tree) }
            ui.onNodeWithTag("include-history").performScrollTo().assertIsOff()
            ui.onNodeWithText("Pairing code").performScrollTo().performTextInput(invite.getString("pairing_code"))
            ui.onNodeWithTag("setup-http").performScrollTo().performClick()
            ui.onNodeWithText("Save").performClick()
            ui.waitUntil(20000) { !model().busy.value && app.store.folders.value.singleOrNull()?.pairingState == "awaiting_confirmation" }
            val folder = app.store.folders.value.single()
            val source = app.store.automatic.source(folder.id)!!
            assertTrue(source.prepared); assertFalse(source.enabled); assertEquals(0, source.waiting)
            assertTrue(app.store.transfers.value.isEmpty())
            // Compose can be idle before Android finishes the dialog-window fade.
            ui.waitUntil(5000) { ui.onAllNodesWithText("Save").fetchSemanticsNodes().isEmpty() }
            android.os.SystemClock.sleep(500)
            saveScreen("pairing-awaiting-confirmation")
            DeviceSupport.setupRequest("/f/${invite.getString("folder_id")}/api/v1/pairing/confirm", invite.getString("receiver_token"),
                org.json.JSONObject().put("verification", folder.verification))
            ui.onNodeWithText("Check pairing").performScrollTo().performClick()
            ui.waitUntil(15000) { !model().busy.value && app.store.folder(folder.id)?.pairingState == "ready" }
            assertFalse(app.store.automatic.source(folder.id)!!.enabled)
            assertTrue(app.store.transfers.value.isEmpty())
            // A file arriving after initialization must not be swallowed by a
            // replacement baseline when Auto is first enabled.
            AutoFixture.put("after-pairing", 17321)
            ui.onNodeWithText("Auto").performClick()
            // This regression exercises the explicitly retained delivery-only
            // baseline workflow, not the new directory initialization mode.
            ui.onNodeWithText("Use delivery Auto").performScrollTo().performClick()
            ui.onNodeWithText("Enable Auto").assertIsEnabled()
            InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("pm grant ${app.packageName} android.permission.POST_NOTIFICATIONS").close()
            ui.onNodeWithText("Enable Auto").performClick()
            ui.waitUntil(15000) { !model().busy.value && app.store.automatic.source(folder.id)?.enabled == true }
            ui.waitUntil(45000) { app.store.transfers.value.singleOrNull()?.status == TransferStatus.UPLOADED }
            assertEquals(17321L, app.store.transfers.value.single().size)
            assertEquals(folder.id, app.store.transfers.value.single().folderId)
            app.auto.pause(folder.id)
            ui.onNodeWithText("Close").performClick()
        } finally { AutoFixture.control("revoke"); AutoFixture.control("reset") }
    }
    @Test fun connectionFailureKeepsNewFolderEditorAndDoesNotSaveOrSend() {
        ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome"))).performClick()
        ui.onNodeWithText("Name").performTextInput("Unreachable")
        ui.onNodeWithText("Server URL").performTextInput(DeviceSupport.server)
        ui.onNodeWithTag("pair-folder").performScrollTo().performClick()
        ui.onNodeWithText("Bearer token").performScrollTo().performTextInput("wrong-token")
        ui.onNodeWithTag("setup-http").performScrollTo().performClick()
        ui.onNodeWithText("Save").performClick()
        ui.waitUntil(15000) { !model().busy.value && model().error.value != null }
        assertTrue(app.store.folders.value.isEmpty()); assertTrue(app.store.transfers.value.isEmpty())
        ui.onNodeWithText("Cancel").assertIsDisplayed().performClick()
    }
    @Test fun multipleFoldersSwitchWithoutMixingTransfers() {
        val first = DeviceSupport.folder("Alpha"); val second = DeviceSupport.folder("Beta")
        DeviceSupport.import(first, name = "Alpha only.bin"); DeviceSupport.import(second, name = "Beta only.bin")
        app.store.refresh(); select(first)
        ui.onNodeWithText("Alpha only.bin").assertIsDisplayed(); ui.onNodeWithText("Beta only.bin").assertDoesNotExist()
        ui.onNodeWithContentDescription("Folders").performClick()
        ui.onNodeWithText("Beta").performClick()
        ui.onNodeWithText("Beta only.bin").assertIsDisplayed(); ui.onNodeWithText("Alpha only.bin").assertDoesNotExist()
    }
    @Test fun systemShareRequiresConfirmationAndCancelEnqueuesNothing() {
        select(DeviceSupport.folder()); share()
        ui.onNodeWithText("Send 1 file(s)").assertIsDisplayed()
        assertTrue(app.store.transfers.value.isEmpty())
        ui.onNodeWithText("Cancel").performClick()
        assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun multipleShareUploadsWithRealWorkerAndDurableReceipts() {
        select(DeviceSupport.folder()); share(true)
        ui.onNodeWithText("Send 2 file(s)").assertIsDisplayed()
        // Keep the runtime permission prompt out of this flow; denial is tested separately.
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("pm grant ${app.packageName} android.permission.POST_NOTIFICATIONS").close()
        ui.onNodeWithText("Send").performClick()
        DeviceSupport.await(60000) { app.store.transfers.value.size == 2 && app.store.transfers.value.all { it.status in listOf(TransferStatus.UPLOADED, TransferStatus.FAILED) } }
        for (file in app.store.transfers.value) {
            assertEquals(file.error, TransferStatus.UPLOADED, file.status)
            assertNotNull(file.deliveryId); assertFalse(File(app.store.directory(file.id), "payload").exists())
        }
        ui.onAllNodesWithContentDescription("Pause transfer").assertCountEquals(0)
        ui.onAllNodesWithContentDescription("Resume or retry transfer").assertCountEquals(0)
    }
    @Test fun textOnlyShareIsRejectedWithoutCrashOrTransfer() {
        select(DeviceSupport.folder())
        app.startActivity(Intent(app, MainActivity::class.java).setAction(Intent.ACTION_SEND).setType("text/plain")
            .putExtra(Intent.EXTRA_TEXT, "not a file").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        ui.waitUntil(5000) { model().error.value != null }
        assertTrue(app.store.transfers.value.isEmpty()); assertTrue(model().pending.value.isEmpty())
    }
    @Test fun invalidShareCountAndColdBusyArrivalAreGuarded() {
        val uris = (0..20).map { DeviceSupport.uri(it + 1) }
        ui.runOnIdle { model().accept(uris) }
        assertTrue(model().pending.value.isEmpty())
        ui.runOnIdle { model().busy.value = true; model().accept(listOf(DeviceSupport.uri())); model().busy.value = false }
        assertEquals(1, model().pending.value.size)
    }
    @Test fun activityRecreationKeepsConfirmedFolderAndPendingShare() {
        select(DeviceSupport.folder()); share()
        ui.onNodeWithText("Send 1 file(s)").assertIsDisplayed()
        val before = activity
        ui.runOnIdle { before.recreate() }
        // onActivityCreated updates our reference before STARTED collection and
        // the dialog's first frame. Wait for the real visible result, not merely
        // a newly allocated Activity, without relaxing pending-state assertions.
        ui.waitUntil(15000) { activity !== before &&
            activity.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED) &&
            ui.onNodeWithText("Send 1 file(s)").isDisplayed() }
        ui.onNodeWithText("Send 1 file(s)").assertIsDisplayed()
        ui.runOnIdle { assertEquals(1, model().pending.value.size) }
        assertEquals(1, app.store.folders.value.size); assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun queuePauseResumeAndStaleOwnerDoNotDuplicateUpload() {
        val folder = DeviceSupport.folder(url = DeviceSupport.server + "/slow")
        val id = DeviceSupport.import(folder, 4 * 1024 * 1024, "pause-resume.bin"); select(folder)
        app.uploads.enqueue(id)
        DeviceSupport.await { app.store.transfer(id)?.uploaded ?: 0 > 0 }
        app.uploads.pause(id)
        assertEquals(TransferStatus.PAUSED, app.store.transfer(id)?.status)
        Thread.sleep(1500)
        assertEquals(TransferStatus.PAUSED, app.store.transfer(id)?.status)
        app.uploads.enqueue(id, manual = true)
        DeviceSupport.await(60000) { app.store.transfer(id)?.status in listOf(TransferStatus.UPLOADED, TransferStatus.FAILED) }
        assertEquals(app.store.transfer(id)?.error, TransferStatus.UPLOADED, app.store.transfer(id)?.status)
    }
    @Test fun transientServerFailureRetriesAutomatically() {
        val folder = DeviceSupport.folder(url = DeviceSupport.server + "/flaky")
        val id = DeviceSupport.import(folder, name = "automatic-retry.bin"); select(folder)
        app.uploads.enqueue(id)
        DeviceSupport.await { app.store.transfer(id)?.error != null }
        assertEquals(TransferStatus.QUEUED, app.store.transfer(id)?.status)
        DeviceSupport.await(90000) { app.store.transfer(id)?.status in listOf(TransferStatus.UPLOADED, TransferStatus.FAILED) }
        assertEquals(app.store.transfer(id)?.error, TransferStatus.UPLOADED, app.store.transfer(id)?.status)
    }
    @Test fun filePickerOpensAndCancelLeavesQueueEmpty() {
        select(DeviceSupport.folder())
        ui.onNodeWithText("Choose files").performClick()
        DeviceSupport.await { device.currentPackageName != app.packageName }
        device.pressBack()
        ui.onNodeWithText("Choose files").assertIsDisplayed()
        assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun landscapeMustKeepExistingTransferVisible() {
        val folder = DeviceSupport.folder()
        app.store.pause(DeviceSupport.import(folder, name = "landscape-visible.bin"))
        app.store.refresh(); select(folder)
        ui.onNodeWithText("landscape-visible.bin").assertIsDisplayed()
        try {
            device.setOrientationLeft()
            ui.waitUntil(10000) { device.displayWidth > device.displayHeight }
            ui.onNodeWithText("landscape-visible.bin").assertIsDisplayed()
            ui.onNodeWithContentDescription("Resume or retry transfer").assertIsDisplayed().assertIsEnabled()
            saveScreen("landscape-fixed")
        } finally {
            device.setOrientationNatural()
            device.unfreezeRotation()
        }
    }
    @Test fun landscapeLargeFontFileAndActionsRemainReachable() {
        val folder = DeviceSupport.folder("A long destination Folder name for large text")
        app.store.pause(DeviceSupport.import(folder, name = "large-font-visible.bin"))
        select(folder)
        withDisplay(fontScale = 1.8f) {
            device.setOrientationLeft()
            ui.waitUntil(10000) { device.displayWidth > device.displayHeight }
            assertTopLeftWordmark()
            ui.onNodeWithText("Auto").assertIsDisplayed()
            ui.onNodeWithContentDescription("Folder settings").assertIsDisplayed()
            assertFileAndFooterReachable("large-font-visible.bin")
            saveScreen("landscape-large-font")
        }
    }
    @Test fun shortWindowEmptyStateCanStillAddFolder() {
        withDisplay(size = "720x640", fontScale = 1.5f) {
            val add = ui.onNode(hasText("Add Folder") and hasAnyAncestor(hasTestTag("welcome")))
            add.performScrollTo()
            saveScreen("short-empty-scrolled")
            add.assertIsDisplayed().performClick()
            ui.onNodeWithText("Save").assertIsDisplayed().assertIsNotEnabled()
            ui.onNodeWithText("Cancel").performClick()
            saveScreen("short-empty")
        }
    }
    @Test fun shortWindowFileAndFooterRemainScrollable() {
        val folder = DeviceSupport.folder()
        app.store.pause(DeviceSupport.import(folder, name = "short-window-visible.bin"))
        select(folder)
        withDisplay(size = "720x640", fontScale = 1.5f) {
            assertFileAndFooterReachable("short-window-visible.bin")
            saveScreen("short-folder")
        }
    }
    @Test fun shortWindowDrawerCanReachLastFolderAndAdd() {
        val folders = (0 until 24).map { DeviceSupport.folder("Destination $it") }
        select(folders.first())
        withDisplay(size = "720x640", fontScale = 1.5f) {
            ui.onNodeWithContentDescription("Folders").performClick()
            ui.onNodeWithTag("folder-drawer").performScrollToNode(hasText("Destination 23"))
            ui.onNodeWithText("Destination 23").performClick()
            ui.runOnIdle { assertEquals(folders.last(), model().selected.value) }
            ui.onNodeWithContentDescription("Folders").performClick()
            ui.onNodeWithTag("folder-drawer").performScrollToNode(hasText("Add Folder"))
            ui.onNodeWithText("Add Folder").performClick()
            ui.onNodeWithText("Name").assertIsDisplayed()
            ui.onNodeWithText("Cancel").performClick()
        }
    }
    @Test fun switchingFromLongListDoesNotCarryScrollIntoAnotherFolder() {
        val first = DeviceSupport.folder("Long list")
        val second = DeviceSupport.folder("Short list")
        for (index in 0 until 60) {
            val id = UUID.randomUUID().toString()
            app.store.addTransfer(id, first, "Row $index.bin", 64)
            app.store.pause(id) // UI-only records never enqueue or require staged payloads.
        }
        app.store.pause(DeviceSupport.import(second, name = "other-folder.bin"))
        select(first)
        ui.onNodeWithTag("folder-content").performScrollToIndex(45)
        ui.onNodeWithContentDescription("Folders").performClick()
        ui.onNodeWithText("Short list").performClick()
        ui.onNodeWithText("Choose files").assertIsDisplayed()
        ui.onNodeWithText("other-folder.bin").assertIsDisplayed()
    }
    private fun assertFileAndFooterReachable(name: String) {
        ui.onNodeWithTag("folder-content").performScrollToNode(hasText(name))
        ui.onNodeWithText(name).assertIsDisplayed()
        ui.onNodeWithContentDescription("Resume or retry transfer").assertIsDisplayed().assertIsEnabled()
        ui.onNodeWithTag("folder-content").performScrollToNode(hasText("Linux delivery receipts", substring = true))
        ui.onNodeWithText("Linux delivery receipts", substring = true).assertIsDisplayed()
        ui.onNodeWithTag("folder-content").performScrollToIndex(0)
        ui.onNodeWithText("Choose files").performScrollTo().assertIsDisplayed().assertIsEnabled()
    }
    private fun withDisplay(size: String? = null, fontScale: Float, block: () -> Unit) {
        val originalFont = device.executeShellCommand("settings get system font_scale").trim()
        val originalSize = Regex("Override size: (\\d+x\\d+)").find(device.executeShellCommand("wm size"))?.groupValues?.get(1)
        try {
            if (size != null) device.executeShellCommand("wm size $size")
            device.executeShellCommand("settings put system font_scale $fontScale")
            ui.waitUntil(15000) { kotlin.math.abs(activity.resources.configuration.fontScale - fontScale) < 0.01f && !model().busy.value }
            ui.waitForIdle()
            block()
        } finally {
            device.setOrientationNatural(); device.unfreezeRotation()
            device.executeShellCommand("wm size ${originalSize ?: "reset"}")
            device.executeShellCommand(if (originalFont == "null") "settings delete system font_scale" else "settings put system font_scale $originalFont")
        }
    }
    private fun saveScreen(name: String) {
        ui.waitForIdle()
        val bitmap = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
        try { writeScreen(bitmap, name) } finally { bitmap.recycle() }
    }
    private fun writeScreen(bitmap: Bitmap, name: String) {
        val directory = File(app.filesDir, "visual-qa").apply { mkdirs() }
        File(directory, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
    @Test fun primaryActionHasVisiblePixelsAfterThemeAndBusyChanges() {
        select(DeviceSupport.folder())
        val night = device.executeShellCommand("cmd uimode night").trim().substringAfter(": ")
        try {
            for ((index, dark) in listOf(false, true, false, true, false, true).withIndex()) {
                device.executeShellCommand("cmd uimode night ${if (dark) "yes" else "no"}")
                ui.waitUntil(15000) {
                    val actual = activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
                    actual == (if (dark) Configuration.UI_MODE_NIGHT_YES else Configuration.UI_MODE_NIGHT_NO) && !model().busy.value
                }
                ui.waitForIdle()
                assertTopLeftWordmark()
                assertActionPixels("Choose files", "theme-$index")
                ui.runOnIdle { model().busy.value = true }
                ui.onNodeWithText("Preparing…").assertIsNotEnabled()
                ui.runOnIdle { model().busy.value = false }
                assertActionPixels("Choose files", "ready-$index")
            }
        } finally {
            device.executeShellCommand("cmd uimode night $night")
        }
    }
    private fun assertActionPixels(label: String, name: String) {
        ui.onNodeWithText(label).assertIsDisplayed().assertIsEnabled()
        // Inspect the actual screen, not just semantics or a forced off-screen redraw.
        val bounds = ui.onNodeWithText(label, useUnmergedTree = true).fetchSemanticsNode().boundsInWindow
        val bitmap = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
        try {
            writeScreen(bitmap, name)
            var bright = 0
            var total = 0
            for (y in bounds.top.toInt().coerceAtLeast(0) until bounds.bottom.toInt().coerceAtMost(bitmap.height)) {
                for (x in bounds.left.toInt().coerceAtLeast(0) until bounds.right.toInt().coerceAtMost(bitmap.width)) {
                    val pixel = bitmap.getPixel(x, y)
                    if (Color.red(pixel) > 180 && Color.green(pixel) > 180 && Color.blue(pixel) > 180) bright++
                    total++
                }
            }
            android.util.Log.i("MiRelayVisualQA", "$name: $bright / $total bright pixels")
            assertTrue("$name: label has no contrasting white ink ($bright / $total pixels, $bounds)",
                total > 0 && bright > total / 50 && bright < total * 3 / 5)
        } finally {
            bitmap.recycle()
        }
    }
}
