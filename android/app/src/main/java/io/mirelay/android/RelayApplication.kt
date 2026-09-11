package io.mirelay.android

import android.app.Application

class RelayApplication : Application() {
    internal val album by lazy { AlbumLibrary(this) }
    val store by lazy { RelayStore(this) }
    val uploads by lazy { UploadQueue(this, store) }
    val auto by lazy { AutoCoordinator(this, store, uploads) }
}
