//! Default names for new layers and objects, in the interface language
//! (spec.md 5.7).
//!
//! A name is document data: once given, it is the person's, and it stays as
//! written when the language changes later. So the language is read once, at
//! creation, from the settings — the same preference the frontend translates
//! with — and the name is written in it. The words match the tool names the
//! catalogues use (`ui/src/i18n/locales`), so an object is called what the
//! palette calls its tool.

use ve_core::schema::ToolKind;

/// The noun for a new layer, in `language`.
fn layer_noun(language: &str) -> &'static str {
    match language {
        "es" => "Capa",
        "fr" => "Calque",
        "de" => "Ebene",
        "it" => "Livello",
        "nl" => "Laag",
        "ja" => "レイヤー",
        "zh" => "图层",
        "ar" => "طبقة",
        _ => "Layer",
    }
}

/// The noun for a new object of `tool`, in `language`.
///
/// Rows follow [`ToolKind`] so a new tool is a compile error in [`index`]
/// rather than an object named in English in eight languages.
fn object_noun(language: &str, tool: ToolKind) -> &'static str {
    const EN: [&str; 13] = [
        "Stroke",
        "Circle",
        "Shape",
        "Mask",
        "Clone",
        "Curve",
        "Intensity",
        "Divergence",
        "Rotation",
        "Warp",
        "Displace",
        "Patch",
        "Macro",
    ];
    const ES: [&str; 13] = [
        "Trazo",
        "Círculo",
        "Forma",
        "Máscara",
        "Clon",
        "Curva",
        "Intensidad",
        "Divergencia",
        "Rotación",
        "Deformación",
        "Desplazamiento",
        "Parche",
        "Macro",
    ];
    const FR: [&str; 13] = [
        "Trait",
        "Cercle",
        "Forme",
        "Masque",
        "Clone",
        "Courbe",
        "Intensité",
        "Divergence",
        "Rotation",
        "Déformation",
        "Déplacement",
        "Patch",
        "Macro",
    ];
    const DE: [&str; 13] = [
        "Strich",
        "Kreis",
        "Form",
        "Maske",
        "Klon",
        "Kurve",
        "Intensität",
        "Divergenz",
        "Drehung",
        "Verformung",
        "Verschiebung",
        "Patch",
        "Makro",
    ];
    const IT: [&str; 13] = [
        "Tratto",
        "Cerchio",
        "Forma",
        "Maschera",
        "Clone",
        "Curva",
        "Intensità",
        "Divergenza",
        "Rotazione",
        "Deformazione",
        "Traslazione",
        "Toppa",
        "Macro",
    ];
    const NL: [&str; 13] = [
        "Streek",
        "Cirkel",
        "Vorm",
        "Masker",
        "Kloon",
        "Curve",
        "Intensiteit",
        "Divergentie",
        "Rotatie",
        "Vervorming",
        "Verschuiving",
        "Patch",
        "Macro",
    ];
    const JA: [&str; 13] = [
        "ストローク",
        "円",
        "形状",
        "マスク",
        "クローン",
        "カーブ",
        "強度",
        "発散",
        "回転",
        "ワープ",
        "ずらし",
        "パッチ",
        "マクロ",
    ];
    const ZH: [&str; 13] = [
        "笔触", "圆形", "形状", "蒙版", "仿制", "曲线", "强度", "辐散", "旋转", "变形", "位移",
        "贴片", "宏",
    ];
    const AR: [&str; 13] = [
        "ضربة",
        "دائرة",
        "شكل",
        "قناع",
        "نسخة",
        "منحنى",
        "شدة",
        "تباعد",
        "دوران",
        "تشويه",
        "إزاحة",
        "رقعة",
        "ماكرو",
    ];
    let row = match language {
        "es" => &ES,
        "fr" => &FR,
        "de" => &DE,
        "it" => &IT,
        "nl" => &NL,
        "ja" => &JA,
        "zh" => &ZH,
        "ar" => &AR,
        _ => &EN,
    };
    row[index(tool)]
}

/// A tool's row in the noun tables. Exhaustive on purpose: no wildcard arm.
fn index(tool: ToolKind) -> usize {
    match tool {
        ToolKind::Brush => 0,
        ToolKind::Circle => 1,
        ToolKind::ShapeFill => 2,
        ToolKind::Mask => 3,
        ToolKind::CloneStamp => 4,
        ToolKind::Curve => 5,
        ToolKind::Intensity => 6,
        ToolKind::Divergence => 7,
        ToolKind::Turn => 8,
        ToolKind::Warp => 9,
        ToolKind::Liquify => 10,
        ToolKind::Patch => 11,
        ToolKind::Macro => 12,
    }
}

/// A duplicate's name: "Stroke 3 copy", in `language`. The original name is
/// the person's and goes in as written.
pub fn copy(language: &str, name: &str) -> String {
    match language {
        "es" | "it" => format!("{name} copia"),
        "fr" => format!("{name} copie"),
        "de" => format!("{name} Kopie"),
        "nl" => format!("{name} kopie"),
        "ja" => format!("{name} のコピー"),
        "zh" => format!("{name} 副本"),
        "ar" => format!("نسخة من {name}"),
        _ => format!("{name} copy"),
    }
}

/// A project given no name, in `language`.
pub fn untitled(language: &str) -> &'static str {
    match language {
        "es" => "Sin título",
        "fr" => "Sans titre",
        "de" => "Unbenannt",
        "it" => "Senza titolo",
        "nl" => "Naamloos",
        "ja" => "無題",
        "zh" => "未命名",
        "ar" => "بدون عنوان",
        _ => "Untitled",
    }
}

/// "Layer 3", in `language`.
pub fn layer(language: &str, number: usize) -> String {
    format!("{} {number}", layer_noun(language))
}

/// "Stroke 3", in `language`.
pub fn object(language: &str, tool: ToolKind, number: usize) -> String {
    format!("{} {number}", object_noun(language, tool))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_names_are_the_ones_projects_always_had() {
        assert_eq!(layer("en", 1), "Layer 1");
        assert_eq!(object("en", ToolKind::Brush, 2), "Stroke 2");
        assert_eq!(object("en", ToolKind::Liquify, 1), "Displace 1");
        // An unknown language is English, as the settings would make it.
        assert_eq!(layer("xx", 4), "Layer 4");
        assert_eq!(copy("en", "Stroke 1"), "Stroke 1 copy");
        assert_eq!(untitled("en"), "Untitled");
    }

    #[test]
    fn every_language_names_every_tool_and_names_them_apart() {
        for language in crate::settings::LANGUAGES {
            let nouns: Vec<_> = ToolKind::ALL
                .iter()
                .map(|tool| object_noun(language, *tool))
                .collect();
            let mut unique = nouns.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), nouns.len(), "{language}: {nouns:?}");
            if *language != "en" {
                assert_ne!(layer_noun(language), "Layer", "{language}");
            }
        }
        assert_eq!(layer("de", 2), "Ebene 2");
        assert_eq!(object("ja", ToolKind::Circle, 1), "円 1");
        assert_eq!(copy("de", "Strich 1"), "Strich 1 Kopie");
        for language in crate::settings::LANGUAGES {
            assert!(copy(language, "N").contains('N'), "{language}");
            assert!(
                *language == "en" || untitled(language) != "Untitled",
                "{language}"
            );
        }
    }
}
