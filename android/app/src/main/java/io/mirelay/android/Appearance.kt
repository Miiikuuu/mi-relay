package io.mirelay.android

import android.app.ActivityManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner

internal data class Appearance(val motion: Boolean = false, val reduceTransparency: Boolean = false)

internal class AppearancePreferences(context: Context) {
    val preferences: SharedPreferences = context.applicationContext.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    fun read(): Appearance {
        val saved = preferences.all
        return Appearance(saved["background_motion"] as? Boolean ?: false, saved["reduce_transparency"] as? Boolean ?: false)
    }
    fun motion(value: Boolean) { preferences.edit { putBoolean("background_motion", value) } }
    fun reduceTransparency(value: Boolean) { preferences.edit { putBoolean("reduce_transparency", value) } }
}

internal val LocalAppearance = staticCompositionLocalOf { Appearance() }
internal data class AppearanceEnvironment(val resumed: Boolean, val powerSave: Boolean, val reducedMotion: Boolean, val lowRam: Boolean)
internal val LocalAppearanceEnvironment = staticCompositionLocalOf { AppearanceEnvironment(false, true, true, true) }

internal fun animateAmbient(appearance: Appearance, environment: AppearanceEnvironment, focused: Boolean, hardware: Boolean) =
    appearance.motion && !appearance.reduceTransparency && environment.resumed && focused && hardware &&
        !environment.powerSave && !environment.reducedMotion && !environment.lowRam

internal fun systemMotionReduced(scale: Float) = !scale.isFinite() || scale <= 0f

@Composable internal fun rememberAppearance(store: AppearancePreferences): Appearance {
    var value by remember(store) { mutableStateOf(store.read()) }
    DisposableEffect(store) {
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> value = store.read() }
        store.preferences.registerOnSharedPreferenceChangeListener(listener)
        value = store.read()
        onDispose { store.preferences.unregisterOnSharedPreferenceChangeListener(listener) }
    }
    return value
}

/** Event-driven; no polling. Recheck on resume, animation-scale changes and
 * power-save broadcasts, including changes made while the app was stopped. */
@Composable internal fun rememberAppearanceEnvironment(): AppearanceEnvironment {
    val context = LocalContext.current.applicationContext
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    fun read(): AppearanceEnvironment = AppearanceEnvironment(
        lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED),
        (context.getSystemService(Context.POWER_SERVICE) as PowerManager).isPowerSaveMode,
        systemMotionReduced(runCatching { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) }.getOrDefault(0f)),
        (context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).isLowRamDevice,
    )
    var value by remember(context, lifecycle) { mutableStateOf(read()) }
    DisposableEffect(context, lifecycle) {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { value = read() }
        }
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) { value = read() }
        }
        val lifecycleObserver = LifecycleEventObserver { _, _ -> value = read() }
        context.contentResolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, observer)
        ContextCompat.registerReceiver(context, receiver, IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
        lifecycle.addObserver(lifecycleObserver)
        value = read()
        onDispose {
            lifecycle.removeObserver(lifecycleObserver)
            context.unregisterReceiver(receiver)
            context.contentResolver.unregisterContentObserver(observer)
        }
    }
    return value
}

@Composable internal fun AppearanceDialog(appearance: Appearance, store: AppearancePreferences, close: () -> Unit) {
    AlertDialog(onDismissRequest = close, containerColor = MaterialTheme.colorScheme.surface, title = { Text("Settings") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()).testTag("appearance-settings"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text("Appearance", style = MaterialTheme.typography.titleSmall)
                AppearanceToggle("Background motion", "Slow light bands. Off by default to save power.", appearance.motion, store::motion, "appearance-motion")
                AppearanceToggle("Reduce transparency", "Use solid surfaces and stop background motion.", appearance.reduceTransparency, store::reduceTransparency, "appearance-transparency")
                if (appearance.reduceTransparency) Text(
                    "Solid appearance is active. Turn off Reduce transparency to see the light bands and translucent panels.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("appearance-solid-explanation"))
                Text("Motion pauses when this window loses focus, in the background, in battery saver, on low-memory devices, or when system animations are off.", style = MaterialTheme.typography.bodySmall)
                Text("These settings apply to all Folders on this device. They do not pause transfers or change Auto sync.", style = MaterialTheme.typography.bodySmall)
            }
        }, confirmButton = { TextButton(onClick = close) { Text("Done") } })
}

@Composable private fun AppearanceToggle(title: String, subtitle: String, value: Boolean, change: (Boolean) -> Unit, tag: String) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag).toggleable(value, role = Role.Switch, onValueChange = change),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(value, onCheckedChange = null)
    }
}
