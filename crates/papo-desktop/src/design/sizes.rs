//! The sizes of particular parts: the rail and channel pane (`docs/design.md`
//! §3 and §5), the server and account screens, their popup and menus (the
//! design project's "Onboarding and Sign In" page). Each is a design value in
//! points; views take them from here, never write their own.

/// The server rail's width, in points.
pub(crate) const RAIL_WIDTH_PT: f32 = 60.0;
/// A server's tile in the rail, in points square.
pub(crate) const RAIL_TILE_PT: f32 = 40.0;
/// `radius.icon`: a rail tile's corner radius, in points.
pub(crate) const RADIUS_ICON_PT: f32 = 12.0;
/// The bar beside the selected server's tile: its width and height, in points.
pub(crate) const RAIL_INDICATOR_PT: (f32, f32) = (4.0, 24.0);
/// `radius.indicator`: the selection bar's corner radius, in points.
pub(crate) const RADIUS_INDICATOR_PT: f32 = 3.0;
/// The outline of the rail's + tile, dashed, in points.
pub(crate) const ADD_TILE_OUTLINE_PT: f32 = 1.0;
/// The + tile's dashes and the gaps between them, in points.
pub(crate) const ADD_TILE_DASH_PT: (f32, f32) = (4.0, 3.0);
/// The + inside its tile, in points square.
pub(crate) const ADD_TILE_PLUS_PT: f32 = 18.0;
/// The channel list's width, in points.
pub(crate) const SIDEBAR_WIDTH_PT: f32 = 232.0;
/// A channel row's height, in points.
pub(crate) const CHANNEL_ROW_HEIGHT_PT: f32 = 32.0;
/// A channel row's # or speaker, in points square.
pub(crate) const CHANNEL_ICON_PT: f32 = 14.0;
/// An icon button inside a pill (⚙), in points square.
pub(crate) const PILL_ICON_PT: f32 = 16.0;
/// The avatar in the server card and the account pill, in points square.
pub(crate) const PILL_AVATAR_PT: f32 = 24.0;
/// The width of the line between panes, in points.
pub(crate) const PANE_EDGE_PT: f32 = 1.0;
/// One row of a menu, in points tall.
pub(crate) const MENU_ROW_HEIGHT_PT: f32 = 32.0;
/// A context menu's width, in points.
pub(crate) const CONTEXT_MENU_WIDTH_PT: f32 = 200.0;
/// The confirm popup's width, in points.
pub(crate) const CONFIRM_WIDTH_PT: f32 = 360.0;

/// The server and account screens' column width, in points, in both hosts.
pub(crate) const FORM_WIDTH_PT: f32 = 340.0;
/// A form field's and a primary button's height, in points.
pub(crate) const FIELD_HEIGHT_PT: f32 = 40.0;
/// A form field's and a primary button's corner radius, in points.
pub(crate) const RADIUS_FIELD_PT: f32 = 10.0;
/// The height of a button inside a field (Show, Hide), in points.
pub(crate) const FIELD_BUTTON_HEIGHT_PT: f32 = 28.0;
/// The width of a field's outline, in points.
pub(crate) const FIELD_OUTLINE_PT: f32 = 1.0;
/// The opacity of a field that cannot take input while a request runs.
pub(crate) const FIELD_BUSY_OPACITY: f32 = 0.6;
/// The opacity of a button that cannot be pressed yet.
pub(crate) const BUTTON_DISABLED_OPACITY: f32 = 0.45;
/// The server's icon above the account steps, in points square.
pub(crate) const SERVER_ICON_PT: f32 = 56.0;
/// The server icon's corner radius, in points.
pub(crate) const RADIUS_SERVER_ICON_PT: f32 = 16.0;
/// A busy button's spinner, in points square.
pub(crate) const SPINNER_PT: f32 = 14.0;
/// The time a busy button's spinner takes to turn once, in seconds.
pub(crate) const SPINNER_PERIOD_S: f64 = 0.8;
/// The popup that hosts the server and account screens in the app, in points.
pub(crate) const POPUP_SIZE_PT: (f32, f32) = (460.0, 580.0);
/// The popup's padding, in points.
pub(crate) const POPUP_PADDING_PT: f32 = 32.0;
/// The popup's corner radius, in points.
pub(crate) const RADIUS_POPUP_PT: f32 = 16.0;
/// The popup's close button, in points square.
pub(crate) const POPUP_CLOSE_PT: f32 = 32.0;
/// The popup's close button's inset from the popup's corner, in points.
pub(crate) const POPUP_CLOSE_INSET_PT: f32 = 10.0;
/// The popup's shadow: its drop and its blur, in points.
pub(crate) const POPUP_SHADOW_PT: (f32, f32) = (24.0, 70.0);
/// A close cross inside its button, in points square.
pub(crate) const CROSS_PT: f32 = 16.0;
/// The close cross in a pill, in points square.
pub(crate) const PILL_CROSS_PT: f32 = 14.0;
/// A keyboard focus ring: its width and its gap from the control, in points.
pub(crate) const FOCUS_RING_PT: (f32, f32) = (2.0, 2.0);
