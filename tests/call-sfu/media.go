// Synthetic publishers share a fixed RTP slot pool. The fixture controls route order and never
// captures host devices.
package main

import (
	"fmt"
	"io"
	"os"
	"sync"
	"time"

	"github.com/pion/rtp"
	"github.com/pion/rtp/codecs"
	"github.com/pion/webrtc/v4"
	"github.com/pion/webrtc/v4/pkg/media"
	"github.com/pion/webrtc/v4/pkg/media/ivfreader"
	"github.com/pion/webrtc/v4/pkg/media/oggreader"
)

const red = "00000000-0000-4000-8000-000000000101"
const green = "00000000-0000-4000-8000-000000000102"
const blue = "00000000-0000-4000-8000-000000000103"

type publisher struct {
	user string
	kind string
}

type videoSlot struct {
	track     *webrtc.TrackLocalStaticRTP
	publisher publisher
	first     *uint16
	next      uint16
	written   bool
	pending   *rtp.Packet
	frame     int
	stamp     uint32
	payloader codecs.VP8Payloader
}

type roomMedia struct {
	mu        sync.Mutex
	revision  uint64
	audioTurn uint64
	swapAudio bool
	rejected  bool
	video     [6]videoSlot
	audio     [8]*webrtc.TrackLocalStaticSample
}

func (m *roomMedia) subscribe(emit func(map[string]any), target publisher, enabled bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if enabled && target == (publisher{blue, "video"}) && !m.rejected {
		m.rejected = true
		emit(map[string]any{"type": "error", "code": "voice-not-found", "message": "Fixture rejected subscription"})
		return
	}
	for i := range m.video {
		slot := &m.video[i]
		if enabled && slot.publisher.user == "" {
			slot.publisher = target
			slot.first = nil
			slot.frame = 0
			if slot.written {
				first := slot.next
				slot.first = &first
			}
			break
		}
		if !enabled && slot.publisher == target {
			slot.publisher = publisher{}
			break
		}
	}
	if enabled && target == (publisher{blue, "video"}) {
		m.swapAudio = true
	}
	m.revision++
	routes := make([]map[string]any, 0, len(m.video))
	for _, slot := range m.video {
		if slot.publisher.user == "" {
			continue
		}
		route := map[string]any{
			"track_id": slot.track.ID(), "user_id": slot.publisher.user, "kind": slot.publisher.kind,
		}
		if slot.first != nil {
			route["first_seq"] = *slot.first
		}
		routes = append(routes, route)
	}
	emit(map[string]any{"type": "voice_video_routes", "revision": m.revision, "routes": routes})
}

func (m *roomMedia) sendSamples(stop <-chan struct{}, emit func(map[string]any)) {
	videoTick := time.NewTicker(100 * time.Millisecond)
	audioTick := time.NewTicker(20 * time.Millisecond)
	defer videoTick.Stop()
	defer audioTick.Stop()
	emit(map[string]any{"type": "voice_audio_routes", "revision": 1, "routes": []map[string]string{
		{"track_id": "papo-audio-0", "user_id": red}, {"track_id": "papo-audio-1", "user_id": green},
	}})
	audioUsers := []string{red, green}
	swapped := false
	for {
		select {
		case <-stop:
			return
		case <-videoTick.C:
			m.sendVideo()
		case <-audioTick.C:
			m.mu.Lock()
			swap := m.swapAudio
			m.mu.Unlock()
			if swap && !swapped {
				swapped = true
				audioUsers[0], audioUsers[1] = audioUsers[1], audioUsers[0]
				emit(map[string]any{"type": "voice_audio_routes", "revision": 2, "routes": []map[string]string{
					{"track_id": "papo-audio-0", "user_id": green}, {"track_id": "papo-audio-1", "user_id": red},
				}})
			}
			if m.audioTurn == 100 {
				emit(map[string]any{"type": "active_speaker_update", "user_ids": []string{green}})
			}
			for i, user := range audioUsers {
				tone := tones[user]
				_ = m.audio[i].WriteSample(media.Sample{Data: tone[m.audioTurn%uint64(len(tone))], Duration: 20 * time.Millisecond})
			}
			m.audioTurn++
		}
	}
}

func (m *roomMedia) sendVideo() {
	m.mu.Lock()
	defer m.mu.Unlock()
	for i := range m.video {
		slot := &m.video[i]
		if slot.publisher.user == "" {
			continue
		}
		// The pending final packet of the previous frame can cross an assignment.
		// The new route's first_seq excludes it even when it arrives after the plan.
		if slot.pending != nil {
			_ = slot.track.WriteRTP(slot.pending)
			slot.pending = nil
		}
		frames := videos[slot.publisher]
		frame := frames[slot.frame%len(frames)]
		slot.frame++
		payloads := slot.payloader.Payload(1150, frame)
		for index, payload := range payloads {
			packet := &rtp.Packet{Header: rtp.Header{
				Version: 2, Marker: index == len(payloads)-1, SequenceNumber: slot.next, Timestamp: slot.stamp,
			}, Payload: payload}
			if packet.Marker {
				slot.pending = packet
			} else {
				_ = slot.track.WriteRTP(packet)
			}
			slot.next++
			slot.written = true
		}
		slot.stamp += 9000
	}
}

func readVideo(path string) ([][]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	reader, _, err := ivfreader.NewWith(file)
	if err != nil {
		return nil, err
	}
	var frames [][]byte
	for {
		data, _, err := reader.ParseNextFrame()
		if err == io.EOF && len(frames) > 0 {
			return frames, nil
		}
		if err != nil {
			return nil, err
		}
		frames = append(frames, data)
	}
}

func readTone(path string) ([][]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	reader, _, err := oggreader.NewWith(file)
	if err != nil {
		return nil, err
	}
	var packets [][]byte
	for {
		data, _, err := reader.ParseNextPage()
		if err == io.EOF && len(packets) > 0 {
			return packets, nil
		}
		if err != nil {
			return nil, err
		}
		if len(data) > 0 && string(data[:min(len(data), 8)]) != "OpusTags" {
			packets = append(packets, data)
		}
	}
}

var videos = map[publisher][][]byte{}
var tones = map[string][][]byte{}

func loadMedia(directory string) error {
	for user, name := range map[publisher]string{
		{red, "video"}: "red", {green, "video"}: "green",
		{blue, "video"}: "magenta", {blue, "screen"}: "blue",
	} {
		data, err := readVideo(fmt.Sprintf("%s/%s.ivf", directory, name))
		if err != nil {
			return fmt.Errorf("load fixture video %s: %w", name, err)
		}
		videos[user] = data
	}
	for user, name := range map[string]string{red: "440", green: "880"} {
		data, err := readTone(fmt.Sprintf("%s/%s.opus", directory, name))
		if err != nil {
			return fmt.Errorf("load fixture audio %s: %w", name, err)
		}
		tones[user] = data
	}
	return nil
}
