package io.github.chriexpe.papo.compose

import android.content.Context
import coil3.ImageLoader
import coil3.gif.AnimatedImageDecoder

/** Shared decoder only. All remote bytes go through MediaCache; animated
 * drawables remain independent so one occurrence cannot pause another. */
internal object MediaImages {
    @Volatile private var shared:ImageLoader?=null
    @Synchronized fun loader(context:Context):ImageLoader=shared ?: ImageLoader.Builder(context.applicationContext)
        .components {add(AnimatedImageDecoder.Factory())}.memoryCache(null).diskCache(null).build().also {shared=it}
}
