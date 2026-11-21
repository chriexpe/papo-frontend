package io.github.chriexpe.papo.videoproof;

import android.app.Activity;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.graphics.Color;
import android.graphics.SurfaceTexture;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.Surface;
import android.view.TextureView;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Displays synthetic SFU video tiles; Surfaces retire independently of stalled native writers. */
public final class ProofActivity extends Activity {
    static {
        System.loadLibrary("papo_android_video_proof");
    }

    private static final String TAG = "PapoVideoProof";
    private static final String ACTION_ARM_STALL = "io.github.chriexpe.papo.videoproof.ACTION_ARM_STALL";
    private static final String ACTION_RELEASE_STALL = "io.github.chriexpe.papo.videoproof.ACTION_RELEASE_STALL";
    private static final String ACTION_RECREATE = "io.github.chriexpe.papo.videoproof.ACTION_RECREATE";
    private static final String ACTION_TRY_OVERFLOW_ATTACH =
            "io.github.chriexpe.papo.videoproof.ACTION_TRY_OVERFLOW_ATTACH";
    private static final String ACTION_DUMP_STATE = "io.github.chriexpe.papo.videoproof.ACTION_DUMP_STATE";

    private static native long nativeAttach(Surface surface);
    private static native long nativeAttachDual(Surface surface0, Surface surface1);
    private static native void nativeDetachGeneration(long generation);
    private static native void nativeAwaitSurfaceRelease(long generation);
    private static native int nativeState();
    private static native int nativeTileState(int index);
    private static native int nativeArmStall();
    private static native void nativeReleaseStall();
    private static native String nativeTelemetryJson();

    private static final class TileSlot {
        final int index;
        final FrameLayout container;
        final TextView fallback;
        TextureView textureView;
        SurfaceBinding binding;

        TileSlot(int index, FrameLayout container, TextView fallback) {
            this.index = index;
            this.container = container;
            this.fallback = fallback;
        }
    }

    private static final class SurfaceBinding {
        final TileSlot slot;
        Surface surface;
        SurfaceTexture destroyedTexture;
        long sessionGen;
        int inFlightAttaches;
        boolean releaseStarted;

        SurfaceBinding(TileSlot slot) {
            this.slot = slot;
        }
    }

    private boolean dual;
    private TileSlot[] tiles;
    private TextView status;
    private boolean started;
    private boolean attached;
    private long attachReqId;
    private long activeGeneration;
    private long lastAttachResult;
    private long lastOverflowAttachResult;
    private int retiredTexturesReleased;
    private long lastRetiredTextureGen;
    private boolean receiverRegistered;

    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            if (!attached) return;
            for (TileSlot tile : tiles) {
                int tState = nativeTileState(tile.index);
                tile.fallback.setVisibility(tState == 1 ? View.INVISIBLE : View.VISIBLE);
            }
            int state = nativeState();
            status.setText(switch (state) {
                case 1 -> dual ? "Dual VP8 · native Surface" : "Fenced VP8 · native Surface";
                case -1 -> "Fixture unavailable";
                default -> {
                    if (dual) {
                        boolean tile0Live = tiles[0].fallback.getVisibility() == View.INVISIBLE;
                        boolean tile1Live = tiles.length > 1
                                && tiles[1].fallback.getVisibility() == View.INVISIBLE;
                        if (tile0Live) {
                            yield "Tile 0 live · Tile 1 waiting";
                        } else if (tile1Live) {
                            yield "Tile 1 live · Tile 0 waiting";
                        }
                    }
                    yield "Waiting for the synthetic camera";
                }
            });
            if (state != -1) status.postDelayed(this, 100);
        }
    };

    private final BroadcastReceiver controlReceiver = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            String action = intent != null ? intent.getAction() : null;
            if (action == null) return;
            switch (action) {
                case ACTION_ARM_STALL -> new Thread(() -> {
                    int armed = nativeArmStall();
                    Log.i(TAG, "ARM_STALL " + armed);
                }, "proof-arm-stall").start();
                case ACTION_RELEASE_STALL -> new Thread(() -> {
                    nativeReleaseStall();
                    Log.i(TAG, "RELEASE_STALL");
                }, "proof-release-stall").start();
                case ACTION_RECREATE -> recreateViews();
                case ACTION_TRY_OVERFLOW_ATTACH -> tryOverflowAttach();
                case ACTION_DUMP_STATE -> dumpState();
                default -> {}
            }
        }
    };

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        dual = getIntent() != null && getIntent().getBooleanExtra("dual", false);
        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        column.setGravity(Gravity.CENTER);
        int padding = Math.round(16 * getResources().getDisplayMetrics().density);
        column.setPadding(padding, padding, padding, padding);
        column.setBackgroundColor(0xff171719);
        column.addView(label("Papo · synthetic SFU fixture", 20));

        tiles = new TileSlot[dual ? 2 : 1];
        tiles[0] = addTile(column, 0, "●  Fixture Red", "Fixture Red · camera", 0xff34292d);
        if (dual) {
            tiles[1] = addTile(column, 1, "●  Fixture Green", "Fixture Green · camera", 0xff253429);
        }

        status = label("Waiting for the synthetic camera", 14);
        column.addView(status);
        setContentView(column);

        IntentFilter filter = new IntentFilter();
        filter.addAction(ACTION_ARM_STALL);
        filter.addAction(ACTION_RELEASE_STALL);
        filter.addAction(ACTION_RECREATE);
        filter.addAction(ACTION_TRY_OVERFLOW_ATTACH);
        filter.addAction(ACTION_DUMP_STATE);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(controlReceiver, filter, Context.RECEIVER_EXPORTED);
        } else {
            registerReceiver(controlReceiver, filter);
        }
        receiverRegistered = true;
    }

    private TileSlot addTile(
            LinearLayout parent, int index, String fallbackText, String caption, int fallbackColor) {
        FrameLayout viewport = new FrameLayout(this);
        parent.addView(viewport, new LinearLayout.LayoutParams(-1, 0, 1));
        FrameLayout container = new FrameLayout(this) {
            @Override protected void onMeasure(int widthSpec, int heightSpec) {
                int width = Math.min(
                        MeasureSpec.getSize(widthSpec), MeasureSpec.getSize(heightSpec) * 16 / 9);
                super.onMeasure(
                        MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                        MeasureSpec.makeMeasureSpec(width * 9 / 16, MeasureSpec.EXACTLY));
            }
        };
        viewport.addView(container, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        TextView fallback = label(fallbackText, dual ? 22 : 26);
        fallback.setBackgroundColor(fallbackColor);
        container.addView(fallback, new FrameLayout.LayoutParams(-1, -1));
        parent.addView(label(caption, 14));

        TileSlot slot = new TileSlot(index, container, fallback);
        installTextureView(slot);
        return slot;
    }

    private void installTextureView(TileSlot slot) {
        if (slot.textureView != null) {
            slot.container.removeView(slot.textureView);
        }
        TextureView view = new TextureView(this);
        SurfaceBinding binding = new SurfaceBinding(slot);
        slot.textureView = view;
        slot.binding = binding;
        view.setSurfaceTextureListener(new TextureView.SurfaceTextureListener() {
            @Override public void onSurfaceTextureAvailable(SurfaceTexture texture, int w, int h) {
                if (slot.binding != binding) {
                    texture.release();
                    return;
                }
                binding.surface = new Surface(texture);
                attachIfReady();
            }

            @Override public void onSurfaceTextureSizeChanged(SurfaceTexture tex, int w, int h) {}

            @Override public void onSurfaceTextureUpdated(SurfaceTexture tex) {}

            @Override public boolean onSurfaceTextureDestroyed(SurfaceTexture texture) {
                binding.destroyedTexture = texture;
                if (slot.binding == binding) {
                    detachActiveSession();
                }
                if (binding.inFlightAttaches == 0) {
                    spawnRetire(binding, binding.sessionGen);
                }
                return false;
            }
        });
        slot.container.addView(view, 0, new FrameLayout.LayoutParams(-1, -1));
    }

    private TextView label(String text, int sizeSp) {
        TextView view = new TextView(this);
        view.setText(text);
        view.setTextSize(sizeSp);
        view.setTextColor(Color.WHITE);
        view.setGravity(Gravity.CENTER);
        return view;
    }

    @Override protected void onStart() {
        super.onStart();
        started = true;
        for (TileSlot tile : tiles) {
            if (tile.binding != null
                    && (tile.binding.sessionGen != 0 || tile.binding.inFlightAttaches > 0)) {
                installTextureView(tile);
            }
        }
        attachIfReady();
    }

    @Override protected void onStop() {
        started = false;
        detachActiveSession();
        super.onStop();
    }

    @Override protected void onDestroy() {
        if (receiverRegistered) {
            unregisterReceiver(controlReceiver);
            receiverRegistered = false;
        }
        super.onDestroy();
    }

    private void attachIfReady() {
        if (!started || attached) return;
        SurfaceBinding[] bindings = new SurfaceBinding[tiles.length];
        Surface[] surfaces = new Surface[tiles.length];
        for (int i = 0; i < tiles.length; i++) {
            SurfaceBinding b = tiles[i].binding;
            if (b == null || b.surface == null || b.destroyedTexture != null) return;
            bindings[i] = b;
            surfaces[i] = b.surface;
        }
        attached = true;
        long reqId = ++attachReqId;
        for (SurfaceBinding b : bindings) {
            b.inFlightAttaches++;
        }
        new Thread(() -> {
            long result = (surfaces.length == 2)
                    ? nativeAttachDual(surfaces[0], surfaces[1])
                    : nativeAttach(surfaces[0]);
            runOnUiThread(() -> {
                for (SurfaceBinding b : bindings) {
                    b.inFlightAttaches--;
                    if (result > 0) {
                        b.sessionGen = result;
                    }
                    if (b.destroyedTexture != null && b.inFlightAttaches == 0) {
                        spawnRetire(b, b.sessionGen);
                    }
                }
                lastAttachResult = result;
                if (reqId != attachReqId || !started) {
                    if (result > 0) {
                        new Thread(() -> nativeDetachGeneration(result), "proof-cancel-stale").start();
                    }
                    return;
                }
                if (result == -1) {
                    status.setText("Native Surface unavailable");
                } else if (result == -2) {
                    status.setText("Retiring session busy");
                } else {
                    activeGeneration = result;
                    refresh.run();
                }
            });
        }, "proof-attach-" + reqId).start();
    }

    private void detachActiveSession() {
        status.removeCallbacks(refresh);
        if (attached) {
            attached = false;
            attachReqId++;
            long oldGen = activeGeneration;
            activeGeneration = 0;
            if (oldGen > 0) {
                new Thread(() -> nativeDetachGeneration(oldGen), "proof-detach-" + oldGen).start();
            }
        }
        for (TileSlot tile : tiles) {
            tile.fallback.setVisibility(View.VISIBLE);
        }
        status.setText("Waiting for the synthetic camera");
    }

    private void spawnRetire(SurfaceBinding binding, long generation) {
        if (binding.releaseStarted || binding.destroyedTexture == null) return;
        binding.releaseStarted = true;
        Surface retiredSurface = binding.surface;
        SurfaceTexture retiredTexture = binding.destroyedTexture;
        binding.surface = null;
        new Thread(() -> {
            if (generation > 0) {
                nativeDetachGeneration(generation);
                nativeAwaitSurfaceRelease(generation);
            }
            if (retiredSurface != null) retiredSurface.release();
            retiredTexture.release();
            runOnUiThread(() -> {
                retiredTexturesReleased++;
                lastRetiredTextureGen = generation;
                Log.i(TAG, "RETIRED_TEXTURE gen=" + generation + " total=" + retiredTexturesReleased);
            });
        }, "proof-retire-" + generation).start();
    }

    private void recreateViews() {
        detachActiveSession();
        for (TileSlot tile : tiles) {
            installTextureView(tile);
        }
    }

    private void tryOverflowAttach() {
        new Thread(() -> {
            SurfaceTexture probeTex = new SurfaceTexture(0);
            Surface probeSurface = new Surface(probeTex);
            long result;
            try {
                result = nativeAttach(probeSurface);
                if (result > 0) {
                    nativeDetachGeneration(result);
                    nativeAwaitSurfaceRelease(result);
                }
            } finally {
                probeSurface.release();
                probeTex.release();
            }
            long recorded = result;
            runOnUiThread(() -> {
                lastOverflowAttachResult = recorded;
                Log.i(TAG, "OVERFLOW_RESULT " + recorded);
            });
        }, "proof-overflow").start();
    }

    private void dumpState() {
        boolean isDual = dual;
        long actGen = activeGeneration;
        long attachRes = lastAttachResult;
        long overflowRes = lastOverflowAttachResult;
        int retiredCount = retiredTexturesReleased;
        long retiredGen = lastRetiredTextureGen;
        new Thread(() -> {
            String nativeJson = nativeTelemetryJson();
            Log.i(TAG, "STATE {\"dual\":" + isDual
                    + ",\"active_gen\":" + actGen
                    + ",\"last_attach_result\":" + attachRes
                    + ",\"last_overflow_attach_result\":" + overflowRes
                    + ",\"retired_textures_released\":" + retiredCount
                    + ",\"last_retired_texture_gen\":" + retiredGen
                    + ",\"native\":" + nativeJson + "}");
        }, "proof-dump-state").start();
    }
}
