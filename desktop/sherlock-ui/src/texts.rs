//! Textes d'interface, en français et au tutoiement, et leur mise en forme : dates, tailles,
//! nombres d'éléments. Les programmes n'écrivent aucun texte d'interface en dur : ils le
//! prennent ici. The `evidence` module holds the texts of Evidence, the file manager; the
//! launcher and Sherlock will reuse them to talk about files.

/// Une date et une heure locales, déjà converties par le programme (fuseau compris).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// De 1 à 12.
    pub month: u8,
    /// De 1 à 31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    /// Jours écoulés depuis le 1er janvier 1970, pour comparer deux dates du calendrier
    /// (algorithme « days from civil » de Howard Hinnant).
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

/// Mois abrégés, comme sur les planches : « 22 nov. ».
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Date courte d'une liste : « aujourd'hui, 21:40 », « hier, 18:02 », « 22 nov. »,
/// « 3 mars 2025 » pour une autre année.
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

/// Date complète : « 22 nov. 2026, 18:02 ».
pub fn full_date(when: LocalTime) -> String {
    let month = MONTHS[usize::from(when.month.clamp(1, 12) - 1)];
    format!(
        "{} {month} {}, {:02}:{:02}",
        when.day, when.year, when.hour, when.minute
    )
}

/// Taille d'un fichier : « 816 octets », « 48 Ko », « 1,5 Mo ». Kilo vaut mille, comme
/// l'écrivent les fiches des disques.
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

/// Contenu d'un dossier : « vide », « 1 élément », « 6 éléments ».
pub fn items(count: usize) -> String {
    match count {
        0 => String::from("vide"),
        1 => String::from("1 élément"),
        n => format!("{n} éléments"),
    }
}

/// Texts of Evidence, the file manager.
pub mod evidence {
    /// Nom de l'application, dans sa barre de titre.
    pub const APP_NAME: &str = "Evidence";

    // Emplacements de la barre latérale.
    pub const FAVORITES: &str = "Favoris";
    pub const SYSTEM: &str = "Système";
    pub const HOME: &str = "Accueil";
    pub const DESKTOP: &str = "Bureau";
    pub const DOCUMENTS: &str = "Documents";
    pub const DOWNLOADS: &str = "Téléchargements";
    pub const PICTURES: &str = "Images";
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
    /// Name of a volume without a label: « SanDisk Cruzer de 16 Go ».
    pub fn model_of(model: &str, size: &str) -> String {
        format!("{model} de {size}")
    }
    /// Name of a volume without label nor model: « Volume de 64 Go ».
    pub fn volume_of(size: &str) -> String {
        format!("Volume de {size}")
    }
    pub const DEVICE_BUSY: &str = "Ce support est occupé : ferme ce qui l'utilise, puis réessaie.";
    pub const DEVICE_NOT_ALLOWED: &str = "Tu n'as pas eu l'autorisation.";
    pub const DEVICE_LOCKED: &str =
        "Ce volume est chiffré. Le déverrouiller ici n'est pas encore possible.";
    pub const DEVICE_UNAVAILABLE: &str = "Les appareils ne répondent pas pour l'instant.";
    pub const DEVICE_FAILED: &str = "Ça n'a pas marché avec ce support.";
    /// Nom du dossier racine dans le fil d'Ariane.
    pub const ROOT: &str = "Disque";
    /// Segments repliés du fil d'Ariane.
    pub const FOLDED: &str = "…";

    // Barre d'outils.
    pub const BACK: &str = "Précédent";
    pub const FORWARD: &str = "Suivant";
    pub fn search_placeholder(folder: &str) -> String {
        format!("Rechercher dans {folder}")
    }
    pub fn branch(name: &str) -> String {
        format!("branche {name}")
    }

    // Boutons de fenêtre.
    pub const MINIMIZE: &str = "Réduire";
    pub const MAXIMIZE: &str = "Agrandir";
    pub const CLOSE: &str = "Fermer";

    // En-têtes de la liste.
    pub const COLUMN_NAME: &str = "Nom";
    pub const COLUMN_MODIFIED: &str = "Modifié";
    pub const COLUMN_SIZE: &str = "Taille";
    pub const COLUMN_KIND: &str = "Type";
    pub const COLUMN_GIT: &str = "Git";
    pub const COLUMN_DELETED: &str = "Supprimé";
    pub const COLUMN_ORIGIN: &str = "Venait de";

    // État git d'un fichier.
    pub const GIT_MODIFIED: &str = "modifié";
    pub const GIT_NEW: &str = "nouveau";
    pub const GIT_IGNORED: &str = "ignoré";

    // Types de fichiers.
    pub const KIND_FOLDER: &str = "Dossier";
    pub const KIND_LINK: &str = "Lien";
    pub const KIND_FILE: &str = "Fichier";
    /// Type d'un fichier d'après son nom exact, avant l'extension.
    pub const KINDS_BY_NAME: [(&str, &str); 7] = [
        ("Dockerfile", "Dockerfile"),
        ("Cargo.lock", "Verrou Cargo"),
        ("justfile", "Recettes just"),
        ("Makefile", "Recettes make"),
        ("LICENSE", "Licence"),
        (".gitignore", "Exclusions git"),
        (".editorconfig", "Réglages d'éditeur"),
    ];
    /// Type d'un fichier d'après son extension, en minuscules.
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
    /// Type d'un fichier à l'extension inconnue : « Fichier XYZ ».
    pub fn kind_from_extension(extension: &str) -> String {
        format!("{KIND_FILE} {}", extension.to_uppercase())
    }

    // Menu du clic droit et raccourcis.
    pub const OPEN: &str = "Ouvrir";
    pub const OPEN_WITH: &str = "Ouvrir avec";
    pub const DEFAULT_APP: &str = "Application par défaut";
    pub const TERMINAL_HERE: &str = "Terminal, dans ce dossier";
    pub const OTHER_APP: &str = "Autre application";
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

    // Noms donnés par l'application.
    /// Nom d'un nouveau dossier, puis « Nouveau dossier 2 »…
    pub fn new_folder_name(n: usize) -> String {
        match n {
            0 | 1 => String::from(NEW_FOLDER),
            n => format!("{NEW_FOLDER} {n}"),
        }
    }
    /// Nom d'une copie quand le nom est pris : « notes (copie).txt », « notes (copie 2).txt ».
    pub fn copy_name(stem: &str, n: usize, extension: Option<&str>) -> String {
        let suffix = match n {
            0 | 1 => String::from(" (copie)"),
            n => format!(" (copie {n})"),
        };
        match extension {
            Some(extension) => format!("{stem}{suffix}.{extension}"),
            None => format!("{stem}{suffix}"),
        }
    }

    // Barre d'état.
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
    pub const DONE: &str = "C'est fait.";
    pub const PATH_COPIED: &str = "Chemin copié.";
    pub const TRASH_EMPTIED: &str = "La corbeille est vide.";

    // Confirmation avant de vider la corbeille.
    pub const EMPTY_TRASH_QUESTION: &str = "Vider la corbeille ?";
    pub fn empty_trash_detail(count: usize) -> String {
        format!(
            "{} de la corbeille seront supprimés pour de bon. Tu ne pourras pas revenir en arrière.",
            match count {
                1 => String::from("L'élément"),
                n => format!("Les {n} éléments"),
            }
        )
    }
    pub const CANCEL: &str = "Annuler";
    pub const CONFIRM_EMPTY: &str = "Vider";

    // Contenu vide.
    pub const EMPTY_FOLDER: &str = "Ce dossier est vide.";
    pub const EMPTY_TRASH_VIEW: &str = "La corbeille est vide.";
    pub const NO_MATCH: &str = "Rien ne correspond à ta recherche.";

    // Aperçu.
    pub fn in_git(size: &str, state: &str) -> String {
        format!("{size}, {state} dans git")
    }
    pub const NO_PREVIEW: &str = "Pas d'aperçu pour ce fichier.";
    pub const NOTHING_SELECTED: &str = "Choisis un fichier pour le voir ici.";
    pub const LOCATION: &str = "Emplacement";

    // Erreurs, dites simplement.
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
    fn les_jours_se_comptent_depuis_1970() {
        assert_eq!(at(1970, 1, 1, 0, 0).days(), 0);
        assert_eq!(at(2000, 3, 1, 0, 0).days(), 11_017);
        assert_eq!(
            at(2026, 1, 1, 0, 0).days() - at(2025, 12, 31, 0, 0).days(),
            1
        );
    }

    #[test]
    fn la_date_courte_suit_les_planches() {
        let now = at(2026, 11, 24, 21, 46);
        assert_eq!(
            short_date(at(2026, 11, 24, 21, 40), now),
            "aujourd'hui, 21:40"
        );
        assert_eq!(short_date(at(2026, 11, 23, 18, 2), now), "hier, 18:02");
        assert_eq!(short_date(at(2026, 11, 22, 9, 0), now), "22 nov.");
        assert_eq!(short_date(at(2025, 3, 3, 9, 0), now), "3 mars 2025");
        // Hier, même à cheval sur deux années.
        assert_eq!(
            short_date(at(2025, 12, 31, 23, 59), at(2026, 1, 1, 0, 1)),
            "hier, 23:59"
        );
    }

    #[test]
    fn les_tailles_sont_en_octets_ko_mo_go() {
        assert_eq!(size(0), "0 octet");
        assert_eq!(size(816), "816 octets");
        assert_eq!(size(1_000), "1 Ko");
        assert_eq!(size(48_400), "48 Ko");
        assert_eq!(size(1_500_000), "1,5 Mo");
        assert_eq!(size(2_000_000_000), "2 Go");
    }

    #[test]
    fn les_noms_de_copie_gardent_l_extension() {
        assert_eq!(
            evidence::copy_name("notes", 1, Some("txt")),
            "notes (copie).txt"
        );
        assert_eq!(evidence::copy_name("src", 3, None), "src (copie 3)");
        assert_eq!(evidence::new_folder_name(2), "Nouveau dossier 2");
        assert_eq!(evidence::status(10, 1), "10 éléments, 1 sélectionné");
    }
}
