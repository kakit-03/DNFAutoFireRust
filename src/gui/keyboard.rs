// Renders the virtual keyboard panel and key layout metadata.

use crate::keymap::parse_single_key;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::EguiApp;
const KEYBOARD_FRAME_WIDTH: f32 = 980.0;
const KEYBOARD_FRAME_HEIGHT: f32 = 240.0;
const KEYBOARD_KEY_WIDTH: f32 = 36.0;
const KEYBOARD_KEY_HEIGHT: f32 = 30.0;
const KEYBOARD_KEY_GAP: f32 = 4.0;
const KEYBOARD_BLOCK_GAP: f32 = 12.0;
const KEYBOARD_MARGIN: f32 = 12.0;
const KEYBOARD_BG_SELECTED: Color32 = Color32::from_rgb(191, 221, 255);
const KEYBOARD_BG_NORMAL: Color32 = Color32::from_rgb(239, 243, 248);
const KEYBOARD_BG_DISABLED: Color32 = Color32::from_rgb(228, 228, 228);
const KEYBOARD_BORDER: Color32 = Color32::from_rgb(120, 132, 148);
const KEYBOARD_TEXT_DISABLED: Color32 = Color32::from_rgb(125, 125, 125);
const KEYBOARD_TEXT_NORMAL: Color32 = Color32::from_rgb(40, 40, 40);

#[derive(Clone, Copy)]
struct KeyboardLayoutKey {
    token: &'static str,
    label: &'static str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct KeyboardCell {
    token: &'static str,
    label: &'static str,
    width_units: f32,
}

fn cell(token: &'static str, label: &'static str, width_units: f32) -> KeyboardCell {
    KeyboardCell {
        token,
        label,
        width_units,
    }
}
impl EguiApp {
    pub(super) fn render_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("按键设置");
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "点击键帽切换是否加入连发。选中的键会保持高亮底色；右上角灰色键仅展示布局，不参与配置。",
                    )
                    .color(Color32::from_rgb(80, 88, 98))
                    .size(13.0),
                );
            });
            ui.add_space(8.0);

            let available = ui.available_size();
            let desired_height = available.y.max(220.0);
            let (response, painter) = ui.allocate_painter(
                Vec2::new(ui.available_width(), desired_height),
                Sense::click(),
            );
            let outer_rect = response.rect.shrink2(Vec2::splat(10.0));
            let scale = (outer_rect.width() / KEYBOARD_FRAME_WIDTH)
                .min(outer_rect.height() / KEYBOARD_FRAME_HEIGHT)
                .max(0.1);
            let scaled_size = Vec2::new(KEYBOARD_FRAME_WIDTH * scale, KEYBOARD_FRAME_HEIGHT * scale);
            let origin = Pos2::new(
                outer_rect.center().x - scaled_size.x / 2.0,
                outer_rect.center().y - scaled_size.y / 2.0,
            );

            let editable = self.state.is_editing_enabled();
            let layout_keys = keyboard_layout_keys();
            let mut hit_token = None;

            if editable && response.clicked() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    for key in &layout_keys {
                        let key_rect = scaled_key_rect(origin, scale, key);
                        if key_rect.contains(pointer_pos) && parse_single_key(key.token).is_ok() {
                            hit_token = Some(key.token.to_string());
                            break;
                        }
                    }
                }
            }

            for key in &layout_keys {
                let supported = parse_single_key(key.token).is_ok();
                let selected = supported && self.state.enabled_keys.contains(key.token);
                let key_rect = scaled_key_rect(origin, scale, key);
                let fill = if selected {
                    KEYBOARD_BG_SELECTED
                } else if supported {
                    KEYBOARD_BG_NORMAL
                } else {
                    KEYBOARD_BG_DISABLED
                };
                let text_color = if supported {
                    KEYBOARD_TEXT_NORMAL
                } else {
                    KEYBOARD_TEXT_DISABLED
                };
                let rounding = egui::CornerRadius::same((6.0 * scale).clamp(3.0, 8.0) as u8);

                painter.rect_filled(key_rect, rounding, fill);
                painter.rect_stroke(
                    key_rect,
                    rounding,
                    Stroke::new(if selected { 1.4 } else { 1.0 }, KEYBOARD_BORDER),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    key_rect.center(),
                    Align2::CENTER_CENTER,
                    key.label,
                    FontId::proportional((12.0 * scale).clamp(8.0, 18.0)),
                    text_color,
                );
            }

            if let Some(token) = hit_token {
                self.state.toggle_enabled_key(&token);
            }
        });
    }
}

fn scaled_key_rect(origin: Pos2, scale: f32, key: &KeyboardLayoutKey) -> Rect {
    Rect::from_min_size(
        Pos2::new(origin.x + key.x * scale, origin.y + key.y * scale),
        Vec2::new(key.width * scale, key.height * scale),
    )
}

fn keyboard_layout_keys() -> Vec<KeyboardLayoutKey> {
    let mut keys = Vec::new();
    let main_x = KEYBOARD_MARGIN;
    let nav_x = main_x + keyboard_units_to_px(16.0) + KEYBOARD_BLOCK_GAP;
    let num_x = nav_x + keyboard_units_to_px(3.0) + KEYBOARD_BLOCK_GAP;

    let row0 = keyboard_row_y(0.0);
    push_keyboard_row(&mut keys, main_x, row0, &[cell("ESC", "Esc", 1.0)]);
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(2.0),
        row0,
        &[
            cell("F1", "F1", 1.0),
            cell("F2", "F2", 1.0),
            cell("F3", "F3", 1.0),
            cell("F4", "F4", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(6.0) + KEYBOARD_BLOCK_GAP,
        row0,
        &[
            cell("F5", "F5", 1.0),
            cell("F6", "F6", 1.0),
            cell("F7", "F7", 1.0),
            cell("F8", "F8", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(10.0) + KEYBOARD_BLOCK_GAP * 2.0,
        row0,
        &[
            cell("F9", "F9", 1.0),
            cell("F10", "F10", 1.0),
            cell("F11", "F11", 1.0),
            cell("F12", "F12", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        row0,
        &[
            cell("PRINTSCREEN", "PrtSc", 1.0),
            cell("SCROLLLOCK", "ScrLk", 1.0),
            cell("PAUSE", "Pause", 1.0),
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(1.0),
        &[
            cell("BACKQUOTE", "~", 1.0),
            cell("1", "1", 1.0),
            cell("2", "2", 1.0),
            cell("3", "3", 1.0),
            cell("4", "4", 1.0),
            cell("5", "5", 1.0),
            cell("6", "6", 1.0),
            cell("7", "7", 1.0),
            cell("8", "8", 1.0),
            cell("9", "9", 1.0),
            cell("0", "0", 1.0),
            cell("MINUS", "-", 1.0),
            cell("EQUAL", "=", 1.0),
            cell("BACKSPACE", "Bksp", 2.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(1.0),
        &[
            cell("INSERT", "Ins", 1.0),
            cell("HOME", "Home", 1.0),
            cell("PAGEUP", "PgUp", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(1.0),
        &[
            cell("NUMLOCK", "Num", 1.0),
            cell("NUMDIV", "/", 1.0),
            cell("NUMMUL", "*", 1.0),
            cell("NUMMINUS", "-", 1.0),
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(2.0),
        &[
            cell("TAB", "Tab", 2.0),
            cell("Q", "Q", 1.0),
            cell("W", "W", 1.0),
            cell("E", "E", 1.0),
            cell("R", "R", 1.0),
            cell("T", "T", 1.0),
            cell("Y", "Y", 1.0),
            cell("U", "U", 1.0),
            cell("I", "I", 1.0),
            cell("O", "O", 1.0),
            cell("P", "P", 1.0),
            cell("LBRACKET", "[", 1.0),
            cell("RBRACKET", "]", 1.0),
            cell("BACKSLASH", "\\", 2.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(2.0),
        &[
            cell("DELETE", "Del", 1.0),
            cell("END", "End", 1.0),
            cell("PAGEDOWN", "PgDn", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(2.0),
        &[
            cell("NUM7", "7", 1.0),
            cell("NUM8", "8", 1.0),
            cell("NUM9", "9", 1.0),
            cell("NUMPLUS", "+", 1.0),
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(3.0),
        &[
            cell("CAPSLOCK", "Caps", 2.0),
            cell("A", "A", 1.0),
            cell("S", "S", 1.0),
            cell("D", "D", 1.0),
            cell("F", "F", 1.0),
            cell("G", "G", 1.0),
            cell("H", "H", 1.0),
            cell("J", "J", 1.0),
            cell("K", "K", 1.0),
            cell("L", "L", 1.0),
            cell("SEMICOLON", ";", 1.0),
            cell("APOSTROPHE", "'", 1.0),
            cell("ENTER", "Enter", 3.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(3.0),
        &[
            cell("NUM4", "4", 1.0),
            cell("NUM5", "5", 1.0),
            cell("NUM6", "6", 1.0),
            cell("NUMENTER", "Ent", 1.0),
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(4.0),
        &[
            cell("LSHIFT", "LShift", 3.0),
            cell("Z", "Z", 1.0),
            cell("X", "X", 1.0),
            cell("C", "C", 1.0),
            cell("V", "V", 1.0),
            cell("B", "B", 1.0),
            cell("N", "N", 1.0),
            cell("M", "M", 1.0),
            cell("COMMA", ",", 1.0),
            cell("PERIOD", ".", 1.0),
            cell("SLASH", "/", 1.0),
            cell("RSHIFT", "RShift", 3.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x + keyboard_units_to_px(1.0),
        keyboard_row_y(4.0),
        &[cell("UP", "Up", 1.0)],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(4.0),
        &[
            cell("NUM1", "1", 1.0),
            cell("NUM2", "2", 1.0),
            cell("NUM3", "3", 1.0),
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(5.0),
        &[
            cell("LCTRL", "LCtrl", 2.0),
            cell("LWIN", "Win", 1.0),
            cell("LALT", "LAlt", 1.0),
            cell("SPACE", "Space", 6.0),
            cell("RALT", "RAlt", 1.0),
            cell("RWIN", "Win", 1.0),
            cell("MENU", "Menu", 1.0),
            cell("RCTRL", "RCtrl", 3.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(5.0),
        &[
            cell("LEFT", "Left", 1.0),
            cell("DOWN", "Down", 1.0),
            cell("RIGHT", "Right", 1.0),
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(5.0),
        &[cell("NUM0", "0", 2.0), cell("NUMDOT", ".", 1.0)],
    );

    keys
}

fn push_keyboard_row(keys: &mut Vec<KeyboardLayoutKey>, base_x: f32, y: f32, row: &[KeyboardCell]) {
    let mut x = base_x;
    for cell in row {
        keys.push(KeyboardLayoutKey {
            token: cell.token,
            label: cell.label,
            x,
            y,
            width: keyboard_units_to_px(cell.width_units),
            height: KEYBOARD_KEY_HEIGHT,
        });
        x += keyboard_units_to_px(cell.width_units) + KEYBOARD_KEY_GAP;
    }
}

fn keyboard_row_y(row: f32) -> f32 {
    KEYBOARD_MARGIN + row * (KEYBOARD_KEY_HEIGHT + KEYBOARD_KEY_GAP)
}

fn keyboard_units_to_px(units: f32) -> f32 {
    units * KEYBOARD_KEY_WIDTH + (units - 1.0) * KEYBOARD_KEY_GAP
}
