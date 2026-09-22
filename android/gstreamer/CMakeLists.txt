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

# Só o que é usado de verdade. A lista não vem de teoria sobre formatos:
# vem dos anexos que o Papo já tem em cache no aparelho — MP4 com H.264 e
# AAC, e os recados de voz em Ogg/Opus.
#
# Ainda não há WebRTC nem câmera: cada fase acrescenta o que usa, e assim o
# que quebra é sempre a última coisa acrescentada.
find_package(GStreamerMobile REQUIRED COMPONENTS
    # --- o que provou o PR anterior, e a base de tudo ---
    coreelements       # fakesink, queue, tee
    app                # appsrc, appsink — o vídeo vira textura por aqui
    playback           # playbin, decodebin
    typefindfunctions  # sem isto o decodebin não sabe o que é o arquivo
    videotestsrc
    audiotestsrc

    # --- conversão que o playbin encaixa sozinho ---
    videoconvertscale
    videorate          # a câmera da call passa por ele
    audioconvert
    audioresample
    audiorate          # costura os buracos da gravação ao vivo
    volume
    level              # o nível de voz: quem está falando

    # --- os recipientes dos anexos de verdade ---
    isomp4             # os .mp4
    ogg                # os recados de voz

    # --- os codecs desses anexos ---
    androidmedia       # H.264 e AAC pelo decodificador do próprio aparelho
    videoparsersbad    # h264parse, que o decodificador exige antes
    audioparsers       # aacparse
    opus               # os recados de voz
    # Reserva em software para o H.264, e só para ele. No aparelho de teste
    # o decodificador de vídeo do próprio aparelho recusa o H.264 logo na
    # primeira tentativa — o OMX da Qualcomm responde "Failed to query
    # component interface for required system resources" — e o decodebin
    # percorre os 54 decodificadores, um a um, até desistir. Sem reserva o
    # vídeo não abre, que era o 0:00/0:00.
    #
    # Só o vídeo precisa disso: o áudio do aparelho funciona (o Opus dos
    # recados decodifica pelo `c2androidopusdecoder`). Por isso `openh264`,
    # que pesa uns poucos MB, e não o `libav` inteiro, que sozinho dobrava
    # o tamanho do APK.
    openh264

    # --- a saída e a entrada de áudio do Android ---
    opensles           # openslessink, e o openslessrc dos recados

    # --- a call ---
    # O motor já existe e funciona na área de trabalho; aqui é só garantir
    # que as peças dele estejam ligadas. No Android a lista é fechada na
    # ligação, e o que falta só aparece em execução como `no element`.
    webrtc             # webrtcbin, o transporte
    nice               # ICE
    dtls               # DTLS
    srtp               # SRTP
    rtpmanager         # a sessão RTP
    rtp                # os pay/depay de Opus e VP8
    vpx                # vp8enc e vp8dec
)
