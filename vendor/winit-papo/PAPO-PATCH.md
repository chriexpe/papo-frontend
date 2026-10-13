# Papo Wayland visibility patch

Based on winit 0.30.13. Changes are limited to its Wayland window state,
visibility methods and redraw scheduling.

Runtime `set_visible(false)` unmaps the toplevel with a null buffer.
`set_visible(true)` restores metadata and requests an initial configure;
`is_visible()` stays false until that configure arrives. Hidden windows still
dispatch explicit redraw requests so eframe's logic-only updates can process
network events and tray commands. They do not draw decorations or request
compositor frame callbacks.

Before the first runtime visibility request, `is_visible()` retains upstream's
`None` result. This preserves eframe's initial hidden-first-frame startup.
The patch deliberately does not change WindowAttributes::visible semantics.

XDG shell reference: https://wayland.app/protocols/xdg-shell#xdg_toplevel
Keep this patch isolated and remove it when upstream supports this lifecycle.
