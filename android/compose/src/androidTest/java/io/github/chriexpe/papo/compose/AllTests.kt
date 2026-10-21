package io.github.chriexpe.papo.compose

import org.junit.runner.RunWith
import org.junit.runners.Suite

@RunWith(Suite::class)
@Suite.SuiteClasses(CoreBridgeTest::class, MediaNetworkTest::class, AttachmentDraftTest::class, AudioWaveformTest::class, ExpressionLibraryTest::class, ComposerDraftTest::class, PapoUiTest::class)
class AllTests
