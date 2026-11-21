// Synthetic publishers share a fixed RTP slot pool. The fixture controls route order and never
// captures host devices.
package main

import (
	"bufio"
	"encoding/binary"
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
	"golang.org/x/sys/unix"
)

const videoRouteURI = "urn:papo:rtp-hdrext:video-route-v1"
const videoRouteExtensionID = 14

const red = "00000000-0000-4000-8000-000000000101"
const green = "00000000-0000-4000-8000-000000000102"
const blue = "00000000-0000-4000-8000-000000000103"

type publisher struct {
	user string
	kind string
}

type videoSlot struct {
	track      *webrtc.TrackLocalStaticRTP
	publisher  publisher
	assignment uint64
	first      *uint16
	next       uint16
	written    bool
	pending    *rtp.Packet
	frame      int
	stamp      uint32
	payloader  codecs.VP8Payloader
}

type emitLogger struct {
	mu     sync.Mutex
	file   *os.File
	writer *bufio.Writer
	lines  uint64
}

func newEmitLogger(path string) (*emitLogger, error) {
	if path == "" {
		return nil, nil
	}
	f, err := os.Create(path)
	if err != nil {
		return nil, err
	}
	return &emitLogger{file: f, writer: bufio.NewWriterSize(f, 32*1024)}, nil
}

func (l *emitLogger) logFrame(pubIdx int, stamp uint32, monoNS uint64) {
	if l == nil {
		return
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.writer == nil {
		return
	}
	_, _ = fmt.Fprintf(l.writer, "%d,%d,%d\n", pubIdx, stamp, monoNS)
	l.lines++
	if l.lines%30 == 0 {
		_ = l.writer.Flush()
	}
}

func (l *emitLogger) flush() {
	if l == nil {
		return
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.writer != nil {
		_ = l.writer.Flush()
	}
}

func (l *emitLogger) close() {
	if l == nil {
		return
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.writer != nil {
		_ = l.writer.Flush()
		l.writer = nil
	}
	if l.file != nil {
		_ = l.file.Close()
		l.file = nil
	}
}

func monotonicNS() uint64 {
	var ts unix.Timespec
	if err := unix.ClockGettime(unix.CLOCK_MONOTONIC, &ts); err != nil {
		return 0
	}
	return uint64(ts.Sec)*1_000_000_000 + uint64(ts.Nsec)
}

func publisherIndex(p publisher) int {
	switch p {
	case publisher{red, "video"}:
		return 0
	case publisher{green, "video"}:
		return 1
	case publisher{blue, "video"}:
		return 2
	case publisher{blue, "screen"}:
		return 3
	default:
		return -1
	}
}

type roomMedia struct {
	mu            sync.Mutex
	startOnce     sync.Once
	profile       string
	slotReuseMode bool
	emit          func(map[string]any)
	emitLog       *emitLogger
	revision      uint64
	assignment    uint64
	audioTurn     uint64
	swapAudio     bool
	rejected      bool
	reuseStaged   bool
	reuseAdvanced bool
	video         [6]videoSlot
	audio         [8]*webrtc.TrackLocalStaticSample
}

func (m *roomMedia) benchMode() bool {
	return m.profile != ""
}

func (m *roomMedia) audioEnabled() bool {
	return m.profile == "" || m.profile == "four-720p"
}

func (m *roomMedia) subscribe(emit func(map[string]any), target publisher, enabled bool) {
	m.mu.Lock()
	m.emit = emit

	if !m.benchMode() && !m.slotReuseMode {
		if enabled && target == (publisher{blue, "video"}) && !m.rejected {
			m.rejected = true
			m.mu.Unlock()
			emit(map[string]any{"type": "error", "code": "voice-not-found", "message": "Fixture rejected subscription"})
			return
		}
	}

	if m.slotReuseMode && enabled && target == (publisher{green, "video"}) && !m.reuseAdvanced {
		// Hold Green until POST /control/reuse/advance-route forces slot 0 rollover.
		routesMsg := m.buildRoutesLocked()
		m.mu.Unlock()
		emit(routesMsg)
		return
	}

	var reused *videoSlot
	for i := range m.video {
		slot := &m.video[i]
		if enabled && slot.publisher == target {
			break
		}
		if enabled && slot.publisher.user == "" {
			slot.publisher = target
			m.assignment++
			slot.assignment = m.assignment
			slot.first = nil
			slot.frame = 0
			if slot.written {
				first := slot.next
				slot.first = &first
				reused = slot
			}
			break
		}
		if !enabled && slot.publisher == target {
			slot.publisher = publisher{}
			break
		}
	}

	if !m.benchMode() && !m.slotReuseMode {
		if enabled && target == (publisher{blue, "video"}) {
			m.swapAudio = true
		}
		// Delay the free snapshot and send a complete replacement keyframe before
		// publishing the assignment, as if the WebSocket were backlogged.
		if !enabled && target == (publisher{red, "video"}) {
			m.mu.Unlock()
			return
		}
	}

	if reused != nil {
		sendVideoFrame(reused, false, nil)
	}
	routesMsg := m.buildRoutesLocked()
	m.mu.Unlock()
	emit(routesMsg)
}

func (m *roomMedia) buildRoutesLocked() map[string]any {
	m.revision++
	routes := make([]map[string]any, 0, len(m.video))
	for _, slot := range m.video {
		if slot.publisher.user == "" {
			continue
		}
		route := map[string]any{
			"track_id": slot.track.ID(), "user_id": slot.publisher.user, "kind": slot.publisher.kind,
			"rtp_route_id": slot.assignment, "rtp_route_ext_id": videoRouteExtensionID,
		}
		if slot.first != nil {
			route["first_seq"] = *slot.first
		}
		routes = append(routes, route)
	}
	return map[string]any{"type": "voice_video_routes", "revision": m.revision, "routes": routes}
}

func (m *roomMedia) stageNewRTP() (map[string]any, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	slot := &m.video[0]
	if slot.publisher != (publisher{red, "video"}) {
		return nil, fmt.Errorf("slot 0 is not holding red video")
	}
	oldAssignment := slot.assignment
	newAssignment := oldAssignment + 1
	if m.assignment < newAssignment {
		m.assignment = newAssignment
	}
	packets := 0
	savedPub := slot.publisher
	savedAssign := slot.assignment
	savedFrame := slot.frame
	slot.publisher = publisher{green, "video"}
	slot.assignment = newAssignment
	slot.frame = 0
	for i := 0; i < 10; i++ {
		packets += sendVideoFrameWithStep(slot, false, 9000, nil)
	}
	slot.publisher = savedPub
	slot.assignment = savedAssign
	slot.frame = savedFrame
	m.reuseStaged = true
	return map[string]any{
		"track_id":       slot.track.ID(),
		"old_assignment": oldAssignment,
		"new_assignment": newAssignment,
		"staged_packets": packets,
	}, nil
}

func (m *roomMedia) advanceReuseRoute() (map[string]any, error) {
	m.mu.Lock()
	slot := &m.video[0]
	if m.emit == nil {
		m.mu.Unlock()
		return nil, fmt.Errorf("no active WebSocket route emitter")
	}
	oldAssignment := slot.assignment
	if oldAssignment == 0 {
		oldAssignment = 1
	}
	newAssignment := oldAssignment + 1
	if m.assignment >= newAssignment {
		newAssignment = m.assignment
	} else {
		m.assignment = newAssignment
	}
	first := slot.next
	slot.first = &first
	slot.publisher = publisher{green, "video"}
	slot.assignment = newAssignment
	slot.frame = 0
	m.reuseAdvanced = true
	routesMsg := m.buildRoutesLocked()
	emitFn := m.emit
	rev := m.revision
	trackID := slot.track.ID()
	m.mu.Unlock()

	emitFn(routesMsg)
	time.Sleep(150 * time.Millisecond)

	m.mu.Lock()
	defer m.mu.Unlock()
	slot = &m.video[0]
	oldPackets := 0
	for i := 0; i < 10; i++ {
		savedPub := slot.publisher
		savedAssign := slot.assignment
		savedFrame := slot.frame
		slot.publisher = publisher{red, "video"}
		slot.assignment = oldAssignment
		slot.frame = 0
		oldPackets += sendVideoFrameWithStep(slot, false, 4500, nil)
		slot.publisher = savedPub
		slot.assignment = savedAssign
		slot.frame = savedFrame
		sendVideoFrameWithStep(slot, false, 4500, nil)
	}
	return map[string]any{
		"track_id":             trackID,
		"revision":             rev,
		"assignment":           newAssignment,
		"injected_old_packets": oldPackets,
	}, nil
}

func (m *roomMedia) sendSamples(stop <-chan struct{}, emit func(map[string]any)) {
	if m.benchMode() {
		m.sendBenchSamples(stop, emit)
		return
	}
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
			m.sendVideo(!m.slotReuseMode)
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
				if len(tone) == 0 {
					continue
				}
				_ = m.audio[i].WriteSample(media.Sample{Data: tone[m.audioTurn%uint64(len(tone))], Duration: 20 * time.Millisecond})
			}
			m.audioTurn++
		}
	}
}

func (m *roomMedia) sendBenchSamples(stop <-chan struct{}, emit func(map[string]any)) {
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		start := time.Now()
		timer := time.NewTimer(0)
		defer timer.Stop()
		if !timer.Stop() {
			select {
			case <-timer.C:
			default:
			}
		}
		for frameIdx := int64(0); ; frameIdx++ {
			target := start.Add(time.Duration(frameIdx) * time.Second / 30)
			if wait := time.Until(target); wait > 0 {
				timer.Reset(wait)
				select {
				case <-stop:
					return
				case <-timer.C:
				}
			} else {
				select {
				case <-stop:
					return
				default:
				}
			}
			m.sendBenchVideo()
		}
	}()

	if m.audioEnabled() {
		emit(map[string]any{"type": "voice_audio_routes", "revision": 1, "routes": []map[string]string{
			{"track_id": "papo-audio-0", "user_id": red}, {"track_id": "papo-audio-1", "user_id": green},
		}})
		wg.Add(1)
		go func() {
			defer wg.Done()
			audioUsers := []string{red, green}
			start := time.Now()
			timer := time.NewTimer(0)
			defer timer.Stop()
			if !timer.Stop() {
				select {
				case <-timer.C:
				default:
				}
			}
			for turn := uint64(0); ; turn++ {
				target := start.Add(time.Duration(turn) * 20 * time.Millisecond)
				if wait := time.Until(target); wait > 0 {
					timer.Reset(wait)
					select {
					case <-stop:
						return
					case <-timer.C:
					}
				} else {
					select {
					case <-stop:
						return
					default:
					}
				}
				for i, user := range audioUsers {
					tone := tones[user]
					if len(tone) == 0 {
						continue
					}
					_ = m.audio[i].WriteSample(media.Sample{Data: tone[turn%uint64(len(tone))], Duration: 20 * time.Millisecond})
				}
			}
		}()
	}

	<-stop
	wg.Wait()
}

func (m *roomMedia) sendBenchVideo() {
	m.mu.Lock()
	defer m.mu.Unlock()
	for i := range m.video {
		slot := &m.video[i]
		if slot.publisher.user == "" {
			continue
		}
		sendVideoFrameWithStep(slot, false, 3000, m.emitLog)
	}
}

func (m *roomMedia) sendVideo(delayLast bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	for i := range m.video {
		slot := &m.video[i]
		if slot.publisher.user == "" {
			continue
		}
		// The final packet of a frame can cross an assignment. It retains the
		// extension written by its own assignment; do not relabel it.
		if slot.pending != nil {
			_ = slot.track.WriteRTP(slot.pending)
			slot.pending = nil
		}
		sendVideoFrame(slot, delayLast, m.emitLog)
	}
}

func sendVideoFrame(slot *videoSlot, delayLast bool, emitLog *emitLogger) int {
	return sendVideoFrameWithStep(slot, delayLast, 9000, emitLog)
}

func sendVideoFrameWithStep(slot *videoSlot, delayLast bool, stampStep uint32, emitLog *emitLogger) int {
	frames := videos[slot.publisher]
	if len(frames) == 0 {
		return 0
	}
	frame := frames[slot.frame%len(frames)]
	slot.frame++
	payloads := slot.payloader.Payload(1150, frame)
	var assignment [8]byte
	binary.BigEndian.PutUint64(assignment[:], slot.assignment)
	sent := 0
	for index, payload := range payloads {
		isLast := index == len(payloads)-1
		packet := &rtp.Packet{Header: rtp.Header{
			Version: 2, Marker: isLast, SequenceNumber: slot.next, Timestamp: slot.stamp,
		}, Payload: payload}
		if err := packet.SetExtension(videoRouteExtensionID, assignment[:]); err != nil {
			return sent
		}
		if packet.Marker && delayLast {
			slot.pending = packet
		} else {
			if err := slot.track.WriteRTP(packet); err == nil && isLast && emitLog != nil {
				if idx := publisherIndex(slot.publisher); idx >= 0 {
					if ts := monotonicNS(); ts > 0 {
						emitLog.logFrame(idx, slot.stamp, ts)
					}
				}
			}
		}
		slot.next++
		slot.written = true
		sent++
	}
	slot.stamp += stampStep
	return sent
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

func loadMedia(directory string, profile string) error {
	var videoMap map[publisher]string
	switch profile {
	case "360p", "720p", "1080p":
		videoMap = map[publisher]string{{red, "video"}: "red"}
	default:
		videoMap = map[publisher]string{
			{red, "video"}: "red", {green, "video"}: "green",
			{blue, "video"}: "magenta", {blue, "screen"}: "blue",
		}
	}
	for user, name := range videoMap {
		data, err := readVideo(fmt.Sprintf("%s/%s.ivf", directory, name))
		if err != nil {
			return fmt.Errorf("load fixture video %s: %w", name, err)
		}
		videos[user] = data
	}
	if profile == "" || profile == "four-720p" {
		for user, name := range map[string]string{red: "440", green: "880"} {
			data, err := readTone(fmt.Sprintf("%s/%s.opus", directory, name))
			if err != nil {
				return fmt.Errorf("load fixture audio %s: %w", name, err)
			}
			tones[user] = data
		}
	}
	return nil
}
