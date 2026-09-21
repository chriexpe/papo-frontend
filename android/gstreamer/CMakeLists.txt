# Monta o `libgstreamer_android.so`: o GStreamer e os plugins escolhidos,
# num arquivo só.
#
# No Android não há `gst-plugin-scanner` nem plugin carregado em tempo de
# execução — o conjunto é decidido aqui, na ligação, e é o que o aparelho
# terá. Este é o caminho que o GStreamer recomenda desde a 1.26; o antigo,
# por `Android.mk` e `ndk-build`, está a caminho da remoção.
cmake_minimum_required(VERSION 3.22)
project(papo_gstreamer LANGUAGES C CXX)

if(NOT DEFINED ENV{GSTREAMER_ROOT_ANDROID})
    message(FATAL_ERROR "GSTREAMER_ROOT_ANDROID não está definido")
endif()

# O SDK traz uma pasta por arquitetura, com nomes que não são os do NDK.
set(_gst_arch_arm64-v8a   "arm64")
set(_gst_arch_armeabi-v7a "armv7")
set(_gst_arch_x86         "x86")
set(_gst_arch_x86_64      "x86_64")
set(GStreamer_ROOT_DIR "$ENV{GSTREAMER_ROOT_ANDROID}/${_gst_arch_${ANDROID_ABI}}")

if(NOT IS_DIRECTORY "${GStreamer_ROOT_DIR}")
    message(FATAL_ERROR "SDK do GStreamer não encontrado em ${GStreamer_ROOT_DIR}")
endif()

list(APPEND CMAKE_MODULE_PATH "${GStreamer_ROOT_DIR}/share/cmake")

# Onde as classes Java dos plugins e os recursos são depositados. Nenhum dos
# plugins abaixo traz Java, mas o módulo exige os caminhos.
set(GStreamer_JAVA_SRC_DIR "${CMAKE_CURRENT_BINARY_DIR}/java")
set(GStreamer_ASSETS_DIR   "${CMAKE_CURRENT_BINARY_DIR}/assets")

# O mínimo para provar que o GStreamer sobe e roda um pipeline. Nada de
# WebRTC, câmera, microfone ou codecs de anexo ainda: cada fase seguinte
# acrescenta o que precisa, e assim o que quebra é sempre a última coisa
# adicionada.
#
#   coreelements       fakesink, queue, tee
#   app                appsrc, appsink
#   playback           decodebin
#   videotestsrc       o pipeline de vídeo do teste
#   audiotestsrc       o de áudio
#   videoconvertscale  videoconvert
#   audioconvert       audioconvert
find_package(GStreamerMobile REQUIRED COMPONENTS
    coreelements
    app
    playback
    videotestsrc
    audiotestsrc
    videoconvertscale
    audioconvert
)
