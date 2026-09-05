package io.mirelay.android

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.net.Uri
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.IntentCompat

class MainActivity : ComponentActivity() {
    private val model: RelayViewModel by viewModels()
    private val picker = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { if (it.isNotEmpty()) model.accept(it) }
    private val notifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }
    private var autoTarget: String? = null
    private val directoryPicker = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        val folder = autoTarget
        autoTarget = null
        if (uri != null && folder != null) model.acceptAutoTree(folder, uri)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        autoTarget = savedInstanceState?.getString("auto_target")
        enableEdgeToEdge()
        if (savedInstanceState == null) acceptIntent(intent)
        setContent {
            RelayScreen(model, chooseFiles = { picker.launch(arrayOf("*/*")) }, chooseDirectory = { folder ->
                autoTarget = folder
                directoryPicker.launch(if (folder == "new-folder") model.creationTree.value else model.autoTree.value)
            }, requestNotifications = {
                if (Build.VERSION.SDK_INT >= 33) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
            })
        }
    }
    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("auto_target", autoTarget)
        super.onSaveInstanceState(outState)
    }
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        acceptIntent(intent)
    }
    private fun acceptIntent(intent: Intent) {
        if (intent.action !in listOf(Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE)) return
        try {
            val uris = when (intent.action) {
                Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
                else -> IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            }.ifEmpty {
                intent.clipData?.let { clip ->
                    if (clip.itemCount > MAX_SHARED_FILES) emptyList() else (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
                }.orEmpty()
            }
            model.accept(uris)
        } catch (_: RuntimeException) {
            model.error.value = "This share could not be read. Try sharing the file again."
        }
    }
}
