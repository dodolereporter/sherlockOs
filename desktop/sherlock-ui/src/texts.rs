//! Interface texts, in French with the informal "tu", and their formatting: dates, sizes,
//! item counts. Programs write no interface text in their code: they take it from here. The
//! `evidence` module holds the texts of Evidence, the file manager; the launcher and Sherlock
//! will reuse them to talk about files.

/// A local date and time, already converted by the program (time zone included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// From 1 to 12.
    pub month: u8,
    /// From 1 to 31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    /// Days elapsed since 1 January 1970, to compare two calendar dates (Howard Hinnant's
    /// "days from civil" algorithm).
    pub fn days(self) -> i64 {
        let month = i64::from(self.month);
        let year = i64::from(self.year) - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let day_of_year =
            (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }
}

/// Abbreviated months, as on the mock-ups: "22 nov.".
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Short date of a listing: "aujourd'hui, 21:40", "hier, 18:02", "22 nov.", "3 mars 2025"
/// for another year.
pub fn short_date(when: LocalTime, now: LocalTime) -> String {
    let clock = format!("{:02}:{:02}", when.hour, when.minute);
    let month = MONTHS[usize::from(when.month.clamp(1, 12) - 1)];
    match now.days() - when.days() {
        0 => format!("aujourd'hui, {clock}"),
        1 => format!("hier, {clock}"),
        _ if when.year == now.year => format!("{} {month}", when.day),
        _ => format!("{} {month} {}", when.day, when.year),
    }
}

/// Full date: "22 nov. 2026, 18:02".
pub fn full_date(when: LocalTime) -> String {
    let month = MONTHS[usize::from(when.month.clamp(1, 12) - 1)];
    format!(
        "{} {month} {}, {:02}:{:02}",
        when.day, when.year, when.hour, when.minute
    )
}

/// Size of a file: "816 octets", "48 Ko", "1,5 Mo". A kilo is a thousand, as disk
/// datasheets write it.
pub fn size(bytes: u64) -> String {
    const KILO: u64 = 1000;
    const MEGA: u64 = KILO * KILO;
    const GIGA: u64 = MEGA * KILO;
    let decimal = |unit: u64, name: &str| {
        let tenths = (bytes * 10 + unit / 2) / unit;
        match tenths % 10 {
            0 => format!("{} {name}", tenths / 10),
            rest => format!("{},{rest} {name}", tenths / 10),
        }
    };
    match bytes {
        0 | 1 => format!("{bytes} octet"),
        b if b < KILO => format!("{b} octets"),
        b if b < MEGA => format!("{} Ko", (b + KILO / 2) / KILO),
        b if b < GIGA => decimal(MEGA, "Mo"),
        _ => decimal(GIGA, "Go"),
    }
}

/// Content of a folder: « vide », « 1 élément », « 6 éléments ».
pub fn items(count: usize) -> String {
    match count {
        0 => String::from("vide"),
        1 => String::from("1 élément"),
        n => format!("{n} éléments"),
    }
}

/// Texts of Evidence, the file manager.
pub mod evidence {
    /// Name of the application, in its title bar.
    pub const APP_NAME: &str = "Evidence";

    // Sidebar places.
    pub const FAVORITES: &str = "Favoris";
    pub const SYSTEM: &str = "Système";
    pub const HOME: &str = "Accueil";
    pub const DESKTOP: &str = "Bureau";
    pub const DOCUMENTS: &str = "Documents";
    pub const DOWNLOADS: &str = "Téléchargements";
    pub const PICTURES: &str = "Images";
    pub const PROJECTS: &str = "Projets";
    pub const TRASH: &str = "Corbeille";
    /// Devices: disks, keys, cards.
    pub const DEVICES: &str = "Appareils";
    pub const SYSTEM_DISK: &str = "Disque système";
    pub const MOUNT: &str = "Monter";
    pub const UNMOUNT: &str = "Démonter";
    pub const EJECT: &str = "Éjecter";
    pub const LOCKED: &str = "Verrouillé";
    pub const EJECTED: &str = "Tu peux le retirer.";
    pub const UNMOUNTED: &str = "Démonté.";
    pub fn free_space(size: &str) -> String {
        format!("{size} libres")
    }
    /// Name of a volume without a label: "SanDisk Cruzer de 16 Go".
    pub fn model_of(model: &str, size: &str) -> String {
        format!("{model} de {size}")
    }
    /// Name of a volume without label nor model: "Volume de 64 Go".
    pub fn volume_of(size: &str) -> String {
        format!("Volume de {size}")
    }
    pub const DEVICE_BUSY: &str = "Ce support est occupé : ferme ce qui l'utilise, puis réessaie.";
    pub const DEVICE_NOT_ALLOWED: &str = "Tu n'as pas eu l'autorisation.";
    pub const DEVICE_LOCKED: &str =
        "Ce volume est chiffré. Le déverrouiller ici n'est pas encore possible.";
    pub const DEVICE_UNAVAILABLE: &str = "Les appareils ne répondent pas pour l'instant.";
    pub const DEVICE_FAILED: &str = "Ça n'a pas marché avec ce support.";
    /// Name of the root folder in the breadcrumb.
    pub const ROOT: &str = "Disque";
    /// Folded breadcrumb segments.
    pub const FOLDED: &str = "…";

    // Toolbar.
    pub const BACK: &str = "Précédent";
    pub const FORWARD: &str = "Suivant";
    pub fn search_placeholder(folder: &str) -> String {
        format!("Rechercher dans {folder}")
    }
    pub fn branch(name: &str) -> String {
        format!("branche {name}")
    }

    // Window buttons.
    pub const MINIMIZE: &str = "Réduire";
    pub const MAXIMIZE: &str = "Agrandir";
    pub const CLOSE: &str = "Fermer";

    // Listing headers.
    pub const COLUMN_NAME: &str = "Nom";
    pub const COLUMN_MODIFIED: &str = "Modifié";
    pub const COLUMN_SIZE: &str = "Taille";
    pub const COLUMN_KIND: &str = "Type";
    pub const COLUMN_GIT: &str = "Git";
    pub const COLUMN_DELETED: &str = "Supprimé";
    pub const COLUMN_ORIGIN: &str = "Venait de";

    // Git status of a file.
    pub const GIT_MODIFIED: &str = "modifié";
    pub const GIT_NEW: &str = "nouveau";
    pub const GIT_IGNORED: &str = "ignoré";

    // File kinds.
    pub const KIND_FOLDER: &str = "Dossier";
    pub const KIND_LINK: &str = "Lien";
    /// Named pipe, socket or device: never opened by the file manager.
    pub const KIND_SPECIAL: &str = "Fichier spécial";
    pub const KIND_FILE: &str = "Fichier";
    /// Kind of a file from its exact name, checked before the extension.
    pub const KINDS_BY_NAME: [(&str, &str); 7] = [
        ("Dockerfile", "Dockerfile"),
        ("Cargo.lock", "Verrou Cargo"),
        ("justfile", "Recettes just"),
        ("Makefile", "Recettes make"),
        ("LICENSE", "Licence"),
        (".gitignore", "Exclusions git"),
        (".editorconfig", "Réglages d'éditeur"),
    ];
    /// Kind of a file from its lowercase extension.
    pub const KINDS_BY_EXTENSION: [(&str, &str); 38] = [
        ("txt", "Texte"),
        ("md", "Markdown"),
        ("toml", "TOML"),
        ("json", "JSON"),
        ("yml", "YAML"),
        ("yaml", "YAML"),
        ("xml", "XML"),
        ("csv", "Tableau CSV"),
        ("log", "Journal"),
        ("conf", "Configuration"),
        ("ini", "Configuration"),
        ("rs", "Rust"),
        ("py", "Python"),
        ("sh", "Script shell"),
        ("js", "JavaScript"),
        ("ts", "TypeScript"),
        ("tsx", "TypeScript"),
        ("jsx", "JavaScript"),
        ("html", "HTML"),
        ("css", "CSS"),
        ("java", "Java"),
        ("c", "C"),
        ("h", "En-tête C"),
        ("cpp", "C++"),
        ("go", "Go"),
        ("sql", "SQL"),
        ("png", "Image PNG"),
        ("jpg", "Image JPEG"),
        ("jpeg", "Image JPEG"),
        ("gif", "Image GIF"),
        ("webp", "Image WebP"),
        ("svg", "Image SVG"),
        ("pdf", "PDF"),
        ("zip", "Archive"),
        ("gz", "Archive"),
        ("mp4", "Vidéo"),
        ("mp3", "Son"),
        ("ttf", "Police"),
    ];
    /// Kind of a file with an unknown extension: « Fichier XYZ ».
    pub fn kind_from_extension(extension: &str) -> String {
        format!("{KIND_FILE} {}", extension.to_uppercase())
    }

    // Right-click menu and shortcuts.
    pub const OPEN: &str = "Ouvrir";
    pub const OPEN_WITH: &str = "Ouvrir avec";
    pub const DEFAULT_APP: &str = "Application par défaut";
    pub const TERMINAL_HERE: &str = "Terminal, dans ce dossier";
    pub const OTHER_APP: &str = "Autre application…";
    /// In the « Ouvrir avec » submenu: the switch that makes the chosen application the default.
    pub const ALWAYS_SHORT: &str = "Toujours ouvrir ce type ainsi";
    /// The same switch in the chooser of every application.
    pub const ALWAYS: &str = "Toujours ouvrir ce type de fichier avec cette application";
    pub const CHOOSER_TITLE: &str = "Ouvrir avec une autre application";
    pub const CHOOSER_SEARCH: &str = "Rechercher une application";
    pub const CHOOSER_EMPTY: &str = "Aucune application ne correspond.";
    pub const DEFAULT_NOT_SAVED: &str = "Ce choix n'a pas pu être enregistré.";
    pub const RENAME: &str = "Renommer";
    pub const DUPLICATE: &str = "Dupliquer";
    pub const COPY_PATH: &str = "Copier le chemin";
    pub const COPY: &str = "Copier";
    pub const CUT: &str = "Couper";
    pub const PASTE: &str = "Coller";
    pub const NEW_FOLDER: &str = "Nouveau dossier";
    pub const SHOW_HIDDEN: &str = "Fichiers cachés";
    pub const MOVE_TO_TRASH: &str = "Mettre à la corbeille";
    pub const RESTORE: &str = "Restaurer";
    pub const EMPTY_TRASH: &str = "Vider la corbeille";
    pub const KEY_OPEN: &str = "Entrée";
    pub const KEY_RENAME: &str = "F2";
    pub const KEY_DUPLICATE: &str = "Ctrl+D";
    pub const KEY_COPY_PATH: &str = "Ctrl+Maj+C";
    pub const KEY_COPY: &str = "Ctrl+C";
    pub const KEY_CUT: &str = "Ctrl+X";
    pub const KEY_PASTE: &str = "Ctrl+V";
    pub const KEY_NEW_FOLDER: &str = "Ctrl+Maj+N";
    pub const KEY_HIDDEN: &str = "Ctrl+H";
    pub const KEY_TRASH: &str = "Suppr";

    // Menus in the bar (lot D-36c).
    pub const MENU_FILE: &str = "Fichier";
    pub const MENU_EDIT: &str = "Édition";
    pub const MENU_VIEW: &str = "Présentation";
    pub const MENU_GO: &str = "Aller";
    pub const MENU_WINDOW: &str = "Fenêtre";
    pub const PARENT_FOLDER: &str = "Dossier parent";
    pub const SHOW_PREVIEW: &str = "Aperçu";
    pub const REFRESH: &str = "Actualiser";
    pub const SELECT_ALL: &str = "Tout sélectionner";
    pub const CLOSE_WINDOW: &str = "Fermer la fenêtre";

    // Names given by the application.
    /// Name of a new folder, then "Nouveau dossier 2"…
    pub fn new_folder_name(n: usize) -> String {
        match n {
            0 | 1 => String::from(NEW_FOLDER),
            n => format!("{NEW_FOLDER} {n}"),
        }
    }
    /// What a copy adds to its name: « (copie) », « (copie 2) »… Kept apart so that names
    /// that are not UTF-8 can be built from their raw bytes.
    pub fn copy_suffix(n: usize) -> String {
        match n {
            0 | 1 => String::from(" (copie)"),
            n => format!(" (copie {n})"),
        }
    }
    /// Name of a copy when the name is taken: "notes (copie).txt", "notes (copie 2).txt".
    pub fn copy_name(stem: &str, n: usize, extension: Option<&str>) -> String {
        let suffix = copy_suffix(n);
        match extension {
            Some(extension) => format!("{stem}{suffix}.{extension}"),
            None => format!("{stem}{suffix}"),
        }
    }

    // Status bar.
    pub fn status(count: usize, selected: usize) -> String {
        let items = super::items(count);
        let items = if count == 0 {
            String::from("Aucun élément")
        } else {
            items
        };
        match selected {
            0 => items,
            1 => format!("{items}, 1 sélectionné"),
            n => format!("{items}, {n} sélectionnés"),
        }
    }
    pub const HINT: &str = "Clic droit pour le menu, Espace pour l'aperçu";
    pub fn copying(count: usize) -> String {
        format!("Copie de {} en cours…", super::items(count))
    }
    pub fn moving(count: usize) -> String {
        format!("Déplacement de {} en cours…", super::items(count))
    }
    pub fn trashed(count: usize) -> String {
        match count {
            1 => String::from("C'est dans la corbeille."),
            n => format!("{n} éléments sont dans la corbeille."),
        }
    }
    pub fn restored(count: usize) -> String {
        match count {
            1 => String::from("C'est revenu à sa place."),
            n => format!("{n} éléments sont revenus à leur place."),
        }
    }
    /// A copy that left special files (sockets, devices) aside.
    pub fn done_skipping(skipped: usize) -> String {
        match skipped {
            1 => String::from(
                "C'est fait. Un fichier spécial (socket ou périphérique) est resté de côté.",
            ),
            n => format!(
                "C'est fait. {n} fichiers spéciaux (sockets ou périphériques) sont restés de côté."
            ),
        }
    }
    pub const DONE: &str = "C'est fait.";
    pub const PATH_COPIED: &str = "Chemin copié.";
    pub const TRASH_EMPTIED: &str = "La corbeille est vide.";

    // Confirmation before emptying the trash.
    pub const EMPTY_TRASH_QUESTION: &str = "Vider la corbeille ?";
    pub fn empty_trash_detail(count: usize) -> String {
        let what = match count {
            1 => String::from("L'élément de la corbeille sera supprimé"),
            n => format!("Les {n} éléments de la corbeille seront supprimés"),
        };
        format!("{what} pour de bon. Tu ne pourras pas revenir en arrière.")
    }
    pub const CANCEL: &str = "Annuler";
    pub const CONFIRM_EMPTY: &str = "Vider";

    // Empty content.
    pub const EMPTY_FOLDER: &str = "Ce dossier est vide.";
    pub const EMPTY_TRASH_VIEW: &str = "La corbeille est vide.";
    pub const NO_MATCH: &str = "Rien ne correspond à ta recherche.";

    // Preview.
    pub fn in_git(size: &str, state: &str) -> String {
        format!("{size}, {state} dans git")
    }
    pub const NO_PREVIEW: &str = "Pas d'aperçu pour ce fichier.";
    pub const NOTHING_SELECTED: &str = "Choisis un fichier pour le voir ici.";
    pub const LOCATION: &str = "Emplacement";

    // Errors, said simply.
    pub fn already_exists(name: &str) -> String {
        format!("« {name} » existe déjà ici.")
    }
    pub const INVALID_NAME: &str = "Ce nom n'est pas possible : évite « / » et les noms vides.";
    pub const INTO_ITSELF: &str = "Tu ne peux pas mettre un dossier dans lui-même.";
    pub const NOT_ALLOWED: &str = "Tu n'as pas le droit de faire ça ici.";
    pub const NOT_FOUND: &str = "Ce fichier n'existe plus.";
    pub const DISK_FULL: &str = "Le disque est plein.";
    pub const OTHER_DISK: &str =
        "Ce fichier est sur un autre disque : la corbeille ne le prend pas encore.";
    pub const NOT_IN_TRASH: &str = "Cet élément n'est plus dans la corbeille.";
    pub const CANNOT_OPEN: &str = "Aucune application n'a pu l'ouvrir.";
    pub const UNREADABLE: &str = "Tu ne peux pas lire ce dossier.";
    pub fn system_error(detail: &str) -> String {
        format!("Le système a refusé : {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            hour,
            minute,
            second: 0,
        }
    }

    #[test]
    fn days_are_counted_from_1970() {
        assert_eq!(at(1970, 1, 1, 0, 0).days(), 0);
        assert_eq!(at(2000, 3, 1, 0, 0).days(), 11_017);
        assert_eq!(
            at(2026, 1, 1, 0, 0).days() - at(2025, 12, 31, 0, 0).days(),
            1
        );
    }

    #[test]
    fn short_date_follows_the_mockups() {
        let now = at(2026, 11, 24, 21, 46);
        assert_eq!(
            short_date(at(2026, 11, 24, 21, 40), now),
            "aujourd'hui, 21:40"
        );
        assert_eq!(short_date(at(2026, 11, 23, 18, 2), now), "hier, 18:02");
        assert_eq!(short_date(at(2026, 11, 22, 9, 0), now), "22 nov.");
        assert_eq!(short_date(at(2025, 3, 3, 9, 0), now), "3 mars 2025");
        // Yesterday, even across two years.
        assert_eq!(
            short_date(at(2025, 12, 31, 23, 59), at(2026, 1, 1, 0, 1)),
            "hier, 23:59"
        );
    }

    #[test]
    fn sizes_are_in_octets_ko_mo_go() {
        assert_eq!(size(0), "0 octet");
        assert_eq!(size(816), "816 octets");
        assert_eq!(size(1_000), "1 Ko");
        assert_eq!(size(48_400), "48 Ko");
        assert_eq!(size(1_500_000), "1,5 Mo");
        assert_eq!(size(2_000_000_000), "2 Go");
    }

    #[test]
    fn copy_names_keep_the_extension() {
        assert_eq!(
            evidence::copy_name("notes", 1, Some("txt")),
            "notes (copie).txt"
        );
        assert_eq!(evidence::copy_name("src", 3, None), "src (copie 3)");
        assert_eq!(evidence::new_folder_name(2), "Nouveau dossier 2");
        assert_eq!(evidence::status(10, 1), "10 éléments, 1 sélectionné");
    }
}
