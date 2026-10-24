# Shared call engine only. Chat media remains on Android Media3.
cmake_minimum_required(VERSION 3.22)
project(papo_call_gstreamer LANGUAGES C CXX)
set(_gst_arch_arm64-v8a arm64)
set(_gst_arch_x86_64 x86_64)
set(GStreamer_ROOT_DIR "$ENV{GSTREAMER_ROOT_ANDROID}/${_gst_arch_${ANDROID_ABI}}")
if(NOT IS_DIRECTORY "${GStreamer_ROOT_DIR}")
    message(FATAL_ERROR "Missing GStreamer Android SDK: ${GStreamer_ROOT_DIR}")
endif()
list(APPEND CMAKE_MODULE_PATH "${GStreamer_ROOT_DIR}/share/cmake")
set(GStreamer_JAVA_SRC_DIR "${CMAKE_CURRENT_BINARY_DIR}/java")
set(GStreamer_ASSETS_DIR "compose/build/gstreamer-assets")
find_package(GStreamerMobile REQUIRED COMPONENTS
    coreelements app autodetect
    audioconvert audioresample audiorate volume level opus opensles
    videoconvertscale videorate videofilter videocrop androidmedia vpx
    webrtc nice dtls srtp rtpmanager rtp
    audiotestsrc videotestsrc
)
