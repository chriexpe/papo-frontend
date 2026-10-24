package io.github.chriexpe.papo.compose

import android.app.Application
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner

/** The connection actor belongs to the process; an Activity disappearing must not end a call. */
class PapoApplication : Application(), ViewModelStoreOwner {
    override val viewModelStore = ViewModelStore()
    val model: PapoViewModel by lazy {
        ViewModelProvider(this, ViewModelProvider.AndroidViewModelFactory.getInstance(this))[
            PapoViewModel::class.java]
    }

    override fun onCreate() {
        super.onCreate()
        AppLanguage.update(this)
        AndroidCallRuntime.initialize(this)
    }
}
