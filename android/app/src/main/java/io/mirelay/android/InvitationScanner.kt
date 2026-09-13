package io.mirelay.android

import android.Manifest
import android.os.Bundle
import android.content.pm.PackageManager
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.CaptureActivity
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

/** Explicit, non-exported capture only. ZXing releases the camera on pause. */
class InvitationScannerActivity : CaptureActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        super.onCreate(savedInstanceState)
    }
}

@Composable
internal fun ScanInvitationButton(enabled: Boolean, accept: (PairingInvitation) -> Unit, failure: (String?) -> Unit) {
    val context = LocalContext.current
    var scanning by remember { mutableStateOf(false) }
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        scanning = false
        if (result.contents != null) {
            try { accept(PairingInvitation.parse(result.contents)); failure(null) }
            catch (error: IllegalArgumentException) { failure(error.message) }
        } else if (result.originalIntent?.getBooleanExtra(Intents.Scan.MISSING_CAMERA_PERMISSION, false) == true) {
            failure("Camera permission is required to scan. You can still enter the invitation manually.")
        }
    }
    fun launchScanner() {
        try {
            scanning = true
            scanner.launch(ScanOptions().setCaptureActivity(InvitationScannerActivity::class.java)
                .setDesiredBarcodeFormats(ScanOptions.QR_CODE).setBeepEnabled(false)
                .setBarcodeImageEnabled(false).setOrientationLocked(false).setTimeout(60_000)
                .setPrompt("Scan the MiRelay invitation on Linux"))
        } catch (_: RuntimeException) {
            scanning = false
            failure("Camera could not open. Try again or enter the invitation manually.")
        }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        scanning = false
        if (granted) launchScanner()
        else failure("Camera permission was not granted. You can still enter the invitation manually.")
    }
    TextButton(enabled = enabled && !scanning, modifier = Modifier.testTag("scan-invitation"), onClick = {
        failure(null)
        if (!context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
            failure("No camera is available. Enter the invitation manually.")
        } else if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            launchScanner()
        } else {
            try { scanning = true; permission.launch(Manifest.permission.CAMERA) }
            catch (_: RuntimeException) { scanning = false; failure("Camera permission could not be requested. Enter the invitation manually.") }
        }
    }) { RelayIcon("scan", null); Spacer(Modifier.width(8.dp)); Text("Scan QR code") }
}
