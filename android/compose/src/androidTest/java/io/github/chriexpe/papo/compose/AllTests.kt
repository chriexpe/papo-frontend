package io.github.chriexpe.papo.compose

import org.junit.runner.RunWith
import org.junit.runners.Suite

@RunWith(Suite::class)
@Suite.SuiteClasses(CoreBridgeTest::class, MediaEditorTest::class, NotificationTest::class, MessageGroupsTest::class, SearchEditorTest::class, MediaCacheTest::class, MediaNetworkTest::class, AttachmentDraftTest::class, AudioWaveformTest::class, ExpressionLibraryTest::class, ComposerDraftTest::class, MessageLinksTest::class, MediaLifecycleTest::class, PapoUiTest::class)
class AllTests
