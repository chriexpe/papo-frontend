package io.github.chriexpe.papo.compose

import org.junit.runner.RunWith
import org.junit.runners.Suite

@RunWith(Suite::class)
@Suite.SuiteClasses(
    LinkRecoveryTest::class,
    CoreBridgeTest::class,
    MediaEditorTest::class,
    NotificationTest::class,
    MessageGroupsTest::class,
    ConversationMarkersTest::class,
    MessagePresentationTest::class,
    SearchEditorTest::class,
    MediaCacheTest::class,
    MediaNetworkTest::class,
    AttachmentDraftTest::class,
    AudioWaveformTest::class,
    ExpressionLibraryTest::class,
    ComposerDraftTest::class,
    ComposerSuggestionsTest::class,
    MarkdownTest::class,
    MessageLinksTest::class,
    MediaLifecycleTest::class,
    AccountAppearanceTest::class,
    PapoUiTest::class,
)
class AllTests
