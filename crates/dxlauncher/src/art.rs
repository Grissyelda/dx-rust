//! The original menu art, loaded from the player's own install (nothing of the game is bundled): DeusExUI.u's
//! UserInterface textures, its bitmap fonts, and the ColorThemeMenu_<name> colours the game's menus use.

use crate::uicanvas::{Rgb, UiFont, UiTex};
use dx_level::TexLib;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use unreal_pkg::props::Value;

/// DeusExPlayer.CreateColorThemeManager's order (ColorThemeMenu_<name>); settings.txt MenuColor indexes it.
const MENU_THEMES: &[&str] = &[
    "Default", "BlueAndGold", "CoolGreen", "Cops", "Cyan", "DesertStorm", "DriedBlood", "Dusk", "Earth", "Green",
    "Grey", "IonStorm", "Lava", "NightVision", "Ninja", "Olive", "PaleGreen", "Pastel", "Plasma", "Primaries",
    "Purple", "Red", "Seawater", "SoylentGreen", "Starlight", "Steel", "SteelGreen", "Superhero", "Terminator",
    "Violet",
];
/// The game's default menu theme (MenuThemeName "Grey").
pub const DEFAULT_THEME: usize = 10;

pub struct Art {
    lib: TexLib,
    tex: HashMap<String, Option<Arc<UiTex>>>,
    /// FontMenuTitle (titles, buttons, values) and FontMenuSmall (help line, edit fields).
    pub title: Arc<UiFont>,
    pub small: Arc<UiFont>,
    /// ColorThemeMenu Colors[]: Background, TitleBackground, TitleText, ButtonFace, ButtonTextNormal,
    /// ButtonTextFocus, ButtonTextDisabled, HelpText, ListText, ListTextHighlight, ListHighlight, ListFocus, Cursor.
    pub colors: [Rgb; 13],
}

impl Art {
    /// None when the folder isn't a Deus Ex install (or its DeusExUI.u can't be read).
    pub fn load(root: &Path, theme: usize) -> Option<Art> {
        let sys = root.join("System");
        let ui = ["DeusExUI.u", "deusexui.u"].iter().map(|f| sys.join(f)).find(|p| p.exists())?;
        let mut lib = TexLib::new(root);
        let title = Arc::new(load_font(&mut lib, &ui, "FontMenuTitle")?);
        let small = Arc::new(load_font(&mut lib, &ui, "FontMenuSmall")?);
        let mut colors = [[255u8; 3]; 13];
        let mut db = unreal_pkg::classes::ClassDb::open(&sys);
        let name = MENU_THEMES.get(theme).copied().unwrap_or("Grey");
        for p in db.defaults(&format!("ColorThemeMenu_{name}")).iter().filter(|p| p.name.eq_ignore_ascii_case("Colors")) {
            if let (Value::Struct(_, b), Some(o)) = (&p.value, colors.get_mut(p.index as usize)) {
                if b.len() >= 3 {
                    *o = [b[0], b[1], b[2]];
                }
            }
        }
        Some(Art { lib, tex: HashMap::new(), title, small, colors })
    }

    /// DeusExUI.UserInterface.<name>.
    pub fn ui(&mut self, name: &str) -> Option<Arc<UiTex>> {
        if let Some(t) = self.tex.get(name) {
            return t.clone();
        }
        let t = self
            .lib
            .load_image_masked(&format!("DeusExUI.UserInterface.{name}"))
            .map(|i| Arc::new(UiTex::from_rgba(i.width, i.height, &i.mips[0])));
        self.tex.insert(name.to_string(), t.clone());
        t
    }

    pub fn col(&self, i: usize) -> Rgb {
        self.colors[i.min(12)]
    }
    /// Button text by state: normal, focus, pressed, disabled.
    pub fn btn(&self, state: usize) -> Rgb {
        self.colors[[4, 5, 5, 6][state.min(3)]]
    }
}

fn load_font(lib: &mut TexLib, file: &Path, name: &str) -> Option<UiFont> {
    let pkg = unreal_pkg::Package::open(file).ok()?;
    let e = pkg
        .exports
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name) && pkg.class_name(e.class_index) == "Font")?;
    let font = unreal_pkg::objects::read_font(&pkg, e).ok()?;
    let mut pages = Vec::new();
    for (tref, _) in &font.pages {
        let t = lib.load_image_masked(&format!("DeusExUI.{}", pkg.object_path(*tref)))?;
        pages.push(Arc::new(UiTex::from_rgba(t.width, t.height, &t.mips[0])));
    }
    Some(UiFont::new(font, pages))
}

/// The menu theme the game last saved (settings.txt MenuColor), else Grey.
pub fn theme_from_settings(kv: &[(String, String)]) -> usize {
    kv.iter()
        .find(|(k, _)| k == "MenuColor")
        .and_then(|(_, v)| v.parse::<f32>().ok())
        .map(|v| v.round().max(0.0) as usize)
        .filter(|v| *v < MENU_THEMES.len())
        .unwrap_or(DEFAULT_THEME)
}

/// settings.txt MenuTranslucency (on by default): the client art adds onto what's behind it.
pub fn translucent_from_settings(kv: &[(String, String)]) -> bool {
    kv.iter().find(|(k, _)| k == "MenuTranslucency").and_then(|(_, v)| v.parse::<f32>().ok()).is_none_or(|v| v > 0.5)
}
