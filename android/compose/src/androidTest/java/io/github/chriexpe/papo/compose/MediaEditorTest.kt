package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File

class MediaEditorTest {
    @Test fun cropGeometryStaysBoundedAndMatchesTheVisibleAspect() {
        val random=kotlin.random.Random(714)
        repeat(1000) {
            val width=random.nextInt(1,10000);val height=random.nextInt(1,10000)
            val aspect=listOf(1f,3f,16f/9,width.toFloat()/height).random(random)
            val crop=mediaCrop(width,height,aspect,random.nextFloat()*8,random.nextFloat()*4-2,random.nextFloat()*4-2)
            assertTrue(crop.x>=-.00001f && crop.y>=-.00001f)
            assertTrue(crop.x+crop.width<=1.00001f && crop.y+crop.height<=1.00001f)
            assertEquals(aspect.toDouble(),(crop.width*width/(crop.height*height)).toDouble(),aspect*.0001)
        }
    }
    @Test fun exportedCropUsesTheSameSourceRegionAsThePreviewAndPreservesTheOriginal() {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val input=File.createTempFile("crop-test-",".png",context.cacheDir)
        val bitmap=android.graphics.Bitmap.createBitmap(400,200,android.graphics.Bitmap.Config.ARGB_8888)
        for(y in 0 until 200) for(x in 0 until 400) bitmap.setPixel(x,y,if(x<200) android.graphics.Color.RED else android.graphics.Color.BLUE)
        input.outputStream().use {bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};bitmap.recycle()
        val original=input.readBytes()
        try {
            val dimensions=inspectMediaImage(input.absolutePath)
            assertEquals(400u,dimensions.width);assertEquals(200u,dimensions.height)
            val crop=mediaCrop(400,200,1f,1f,1f,.5f)
            val output=prepareMediaImage(input.absolutePath,crop,MediaEditPurpose.AVATAR)
            assertEquals(200u,output.width);assertEquals(200u,output.height);assertTrue(output.bytes<=2uL*1024u*1024u)
            val bytes=android.util.Base64.decode(output.blob,android.util.Base64.DEFAULT)
            val exported=android.graphics.BitmapFactory.decodeByteArray(bytes,0,bytes.size)
            assertEquals(android.graphics.Color.BLUE,exported.getPixel(100,100));exported.recycle()
            assertArrayEquals(original,input.readBytes())
        } finally {input.delete()}
    }
}
