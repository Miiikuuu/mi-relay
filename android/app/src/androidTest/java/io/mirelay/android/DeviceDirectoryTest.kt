package io.mirelay.android

import android.app.Activity
import android.app.Application
import android.content.Intent
import android.os.Bundle
import android.util.Base64
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.*
import org.junit.Assert.*
import org.junit.runner.RunWith
import java.io.File

/** Real SAF provider → JNI/tus → isolated server → Linux directory CLI. */
@RunWith(AndroidJUnit4::class)
class DeviceDirectoryTest {
    @get:Rule val ui=createEmptyComposeRule()
    private val app get()=DeviceSupport.app
    private lateinit var activity:MainActivity
    private fun model()=ViewModelProvider(activity)[RelayViewModel::class.java]
    private val lifecycle=object:Application.ActivityLifecycleCallbacks {
        override fun onActivityCreated(a:Activity,s:Bundle?){if(a is MainActivity)activity=a}
        override fun onActivityStarted(a:Activity){};override fun onActivityResumed(a:Activity){}
        override fun onActivityPaused(a:Activity){};override fun onActivityStopped(a:Activity){}
        override fun onActivitySaveInstanceState(a:Activity,s:Bundle){};override fun onActivityDestroyed(a:Activity){}
    }
    @Before fun setup(){
        DeviceSupport.reset();AutoFixture.reset();app.registerActivityLifecycleCallbacks(lifecycle)
        InstrumentationRegistry.getInstrumentation().startActivitySync(Intent(app,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
        ui.waitUntil(15000){!model().busy.value}
    }
    @After fun close(){
        app.store.automatic.sources.value.filter{it.enabled}.forEach{app.auto.pause(it.folderId)}
        AutoFixture.control("revoke");AutoFixture.control("reset")
        InstrumentationRegistry.getInstrumentation().runOnMainSync {if(::activity.isInitialized)activity.finish()}
        app.unregisterActivityLifecycleCallbacks(lifecycle)
    }
    private data class Pairing(val folder:String,val identity:String,val receiver:String)
    private fun pair():Pairing {
        val invite=DeviceSupport.invitation();val connection=FolderConnection(app);val sender=connection.newSenderToken()
        val info=connection.claim(DeviceSupport.server,invite.getString("pairing_code"),sender,true)
        DeviceSupport.setupRequest("/f/${invite.getString("folder_id")}/api/v1/pairing/confirm",invite.getString("receiver_token"),JSONObject().put("verification",info.getString("verification")))
        val key=app.store.saveFolder(null,"Directory QA",connection.folderUrl(DeviceSupport.server,invite.getString("pairing_code")),sender,true,"ready")
        return Pairing(key,invite.getString("folder_id"),invite.getString("receiver_token"))
    }
    private fun linux(pair:Pairing,action:String="receive",files:List<Pair<String,ByteArray>> = emptyList()):JSONObject =
        DeviceSupport.setupRequest("/__directory/${pair.identity}/$action",pair.receiver,JSONObject().put("files",JSONArray().apply {files.forEach{put(JSONObject().put("path",it.first).put("data",Base64.encodeToString(it.second,Base64.NO_WRAP)))}}))
    private fun bytes(size:Int,offset:Int=0)=ByteArray(size){((it+offset)%251).toByte()}
    private fun sha(bytes:ByteArray)=java.security.MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
    private fun scan(folder:String,time:Long)=AutoSession().use{app.auto.scan(app.store.automatic.source(folder)!!,it){time}}
    private fun uploaded(count:Int)=DeviceSupport.await(90000){app.store.refresh();app.store.transfers.value.let{it.size==count && it.all{row->row.status==TransferStatus.UPLOADED}}}
    private fun screenshot(name:String){
        ui.waitForIdle();android.os.SystemClock.sleep(250)
        val bitmap=InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()!!
        val directory=File(app.filesDir,"visual-qa");directory.mkdirs()
        File(directory,"$name.png").outputStream().use{bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};bitmap.recycle()
    }

    @Test fun previewUiInitializesExistingDirectoryAndAutoSyncsUpdatesConflictsAndOfflineChanges(){
        val pair=pair();val local="Linux original".toByteArray()
        AutoFixture.put("same",96,extra={putString("name","same.bin")})
        AutoFixture.put("dir",extra={putBoolean("directory",true);putString("name","Pixiv")})
        AutoFixture.put("nested",16384,"dir",extra={putString("name"," original.bin")})
        AutoFixture.put("copy",16384,extra={putString("name","copy.bin")})
        AutoFixture.put("different",256,extra={putString("name","different.bin")})
        linux(pair,"prepare",listOf("same.bin" to bytes(96),"different.bin" to local,"linux-only.txt" to "keep".toByteArray()))
        val preview=app.auto.previewDirectory(pair.folder,AutoFixture.tree)
        assertEquals(1,preview.identical);assertEquals(2,preview.missing.size);assertEquals(1,preview.different.size);assertEquals(1,preview.destinationOnly)
        assertTrue(app.store.transfers.value.isEmpty());assertNull(app.store.automatic.source(pair.folder))
        ui.runOnIdle{model().selected.value=pair.folder;model().openAuto(pair.folder)}
        ui.waitUntil(15000){!model().busy.value}
        ui.onNodeWithText("Directory sync").assertIsDisplayed()
        ui.onNodeWithText("1 identical · 2 missing · 1 different").performScrollTo().assertIsDisplayed()
        screenshot("directory-preview")
        ui.onNodeWithText("Initialize sync").performClick()
        ui.waitUntil(30000){!model().busy.value && app.store.automatic.sources.value.any{it.folderId==pair.folder && it.directorySync}}
        uploaded(3)
        assertTrue(app.store.transfers.value.all{it.relativePath!=null && !it.received})
        ui.onNodeWithText("Close").performClick()
        assertTrue(ui.onAllNodes(hasText("Waiting for Linux",substring=true)).fetchSemanticsNodes().isNotEmpty())
        screenshot("directory-waiting-linux")
        val received=linux(pair)
        val paths=received.getJSONArray("files").let{a->(0 until a.length()).associate{val f=a.getJSONObject(it);f.getString("path") to f.getString("sha256")}}
        assertEquals(sha(bytes(16384)),paths["Pixiv/ original.bin"]);assertEquals(paths["copy.bin"],paths["Pixiv/ original.bin"])
        assertEquals(sha(local),received.getJSONObject("receipts").getJSONObject("different.bin").getString("history_sha256"))
        app.auto.refreshReceipts(pair.folder);assertTrue(app.store.transfers.value.all{it.received});assertTrue(app.store.transfers.value.single{it.relativePath=="different.bin"}.conflict)
        ui.waitForIdle();screenshot("directory-synced")
        // Same size and modification timestamp, but different bytes.
        AutoFixture.put("nested",16384,"dir",extra={putString("name"," original.bin");putInt("contentOffset",1)})
        var time=System.currentTimeMillis()+20000;scan(pair.folder,time);scan(pair.folder,time+11000);uploaded(4)
        val updated=linux(pair);assertFalse(updated.getJSONObject("receipts").getJSONObject("Pixiv/ original.bin").getBoolean("conflict"))
        val linuxEdit="Linux edit".toByteArray();linux(pair,"edit",listOf("Pixiv/ original.bin" to linuxEdit))
        AutoFixture.put("nested",16384,"dir",extra={putString("name"," original.bin");putInt("contentOffset",2)})
        time+=40000;scan(pair.folder,time);scan(pair.folder,time+11000);uploaded(5)
        val conflict=linux(pair).getJSONObject("receipts").getJSONObject("Pixiv/ original.bin")
        assertTrue(conflict.getBoolean("conflict"));assertEquals(sha(linuxEdit),conflict.getString("history_sha256"))
        AutoFixture.control("remove",Bundle().apply{putString("id","copy")});scan(pair.folder,time+30000)
        assertTrue(linux(pair).getJSONArray("files").toString().contains("copy.bin"));assertEquals(5,app.store.transfers.value.size)
        app.auto.pause(pair.folder)
        AutoFixture.put("offline",71,extra={putString("name","offline.bin")})
        app.auto.resumeDirectory(pair.folder,AutoFixture.tree,true);uploaded(6)
        assertTrue(linux(pair).getJSONArray("files").toString().contains("offline.bin"))
        app.auto.refreshReceipts(pair.folder);assertTrue(app.store.transfers.value.all{it.received || it.superseded})
    }
    @Test fun changedPreviewCannotEnableAndUnsupportedPathsNeverUpload(){
        val pair=pair();linux(pair,"prepare");AutoFixture.put("file",64,extra={putString("name","first.bin")})
        val preview=app.auto.previewDirectory(pair.folder,AutoFixture.tree)
        AutoFixture.put("file",64,extra={putString("name","first.bin");putInt("contentOffset",1)})
        assertThrows(IllegalArgumentException::class.java){app.auto.confirmDirectory(pair.folder,AutoFixture.tree,preview.id,true)}
        assertNull(app.store.automatic.source(pair.folder));assertTrue(app.store.transfers.value.isEmpty())
        AutoFixture.put("bad",64,extra={putString("name","../escape.bin")})
        assertThrows(IllegalArgumentException::class.java){app.auto.previewDirectory(pair.folder,AutoFixture.tree)}
        assertNull(app.store.automatic.source(pair.folder));assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun providerChangesDuringHashingAndIncompleteListingsNeverInitialize(){
        val pair=pair();linux(pair,"prepare")
        AutoFixture.put("unstable",100,extra={putBoolean("mutateOnRead",true)})
        assertThrows(Exception::class.java){app.auto.previewDirectory(pair.folder,AutoFixture.tree)}
        AutoFixture.put("unstable",100)
        for(mode in listOf("loading","error","duplicate")) {
            AutoFixture.mode(mode);assertThrows(Exception::class.java){app.auto.previewDirectory(pair.folder,AutoFixture.tree)}
        }
        assertTrue(app.store.transfers.value.isEmpty());assertNull(app.store.automatic.source(pair.folder))
    }
    @Test fun legacyFoldersCannotSilentlyBecomeDirectorySync(){
        val folder=DeviceSupport.folder();AutoFixture.put("old")
        assertThrows(IllegalArgumentException::class.java){app.auto.previewDirectory(folder,AutoFixture.tree)}
        assertNull(app.store.automatic.source(folder));assertTrue(app.store.transfers.value.isEmpty())
    }
    @Test fun initialBacklogContinuesAcrossBoundedBatchesWithoutManualChecks(){
        val pair=pair();linux(pair,"prepare")
        repeat(45){i->AutoFixture.put("file-$i",32,extra={putString("name","file-$i.bin")})}
        val preview=app.auto.previewDirectory(pair.folder,AutoFixture.tree)
        app.auto.confirmDirectory(pair.folder,AutoFixture.tree,preview.id,true)
        uploaded(45)
        val received=linux(pair);assertEquals(45,received.getJSONArray("files").length())
        app.auto.refreshReceipts(pair.folder);assertTrue(app.store.transfers.value.all{it.received})
        assertEquals(45,app.store.transfers.value.map{it.sourceVersion}.distinct().size)
    }
}
