//! Prototype jetable du lot D-00 : la barre du haut en Slint, ancrée par layer-shell.
//!
//! Slint ne sait pas créer de surface layer-shell : son backend winit ne connaît que les
//! fenêtres ordinaires. Ce prototype branche donc Slint sur Wayland lui-même, par l'API
//! publique `slint::platform` : smithay-client-toolkit crée la surface layer-shell et les
//! `xdg_popup` imbriqués du menu, et le moteur de rendu logiciel de Slint dessine dans des
//! tampons `wl_shm`. C'est le travail qu'un shell en Slint devrait reprendre et entretenir.

slint::include_modules!();

mod wayland;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PlatformError, Timer, TimerMode};

/// Ce que l'interface demande à la boucle Wayland. Les rappels de Slint ne touchent jamais
/// directement aux surfaces : ils déposent une commande, traitée au tour de boucle suivant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleMenu,
    OpenSubMenu,
    CloseSubMenu,
    CloseMenus,
    SelectDesktop(i32),
    Exit,
}

pub type Commands = Rc<RefCell<VecDeque<Command>>>;

/// Plateforme Slint minimale : chaque composant reçoit une fenêtre à rendu logiciel. La boucle
/// Wayland la rattache ensuite à une surface layer-shell ou à un popup.
struct WaylandPlatform {
    last_created: Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>,
}

impl Platform for WaylandPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        *self.last_created.borrow_mut() = Some(window.clone());
        Ok(window)
    }
}

/// Les trois composants Slint et la fenêtre logicielle de chacun.
pub struct Ui {
    pub bar: Bar,
    pub menu: Menu,
    pub submenu: SubMenu,
    pub bar_window: Rc<MinimalSoftwareWindow>,
    pub menu_window: Rc<MinimalSoftwareWindow>,
    pub submenu_window: Rc<MinimalSoftwareWindow>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let last_created = Rc::new(RefCell::new(None));
    slint::platform::set_platform(Box::new(WaylandPlatform {
        last_created: last_created.clone(),
    }))
    .map_err(|e| format!("plateforme Slint : {e}"))?;

    let take_window = || {
        last_created
            .borrow_mut()
            .take()
            .ok_or("Slint n'a pas créé de fenêtre")
    };
    let bar = Bar::new()?;
    let bar_window = take_window()?;
    let menu = Menu::new()?;
    let menu_window = take_window()?;
    let submenu = SubMenu::new()?;
    let submenu_window = take_window()?;

    let commands: Commands = Rc::new(RefCell::new(VecDeque::new()));
    wire_callbacks(&bar, &menu, &submenu, &commands);

    let (clock, date) = now_strings();
    bar.set_clock(clock.into());
    bar.set_date(date.into());
    let clock_timer = Timer::default();
    let weak_bar = bar.as_weak();
    clock_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        if let Some(bar) = weak_bar.upgrade() {
            let (clock, date) = now_strings();
            bar.set_clock(clock.into());
            bar.set_date(date.into());
        }
    });

    let _auto = std::env::var_os("ESSAI_AUTO").map(|_| start_auto(commands.clone()));

    let ui = Ui {
        bar,
        menu,
        submenu,
        bar_window,
        menu_window,
        submenu_window,
    };
    wayland::run(ui, commands)
}

fn wire_callbacks(bar: &Bar, menu: &Menu, submenu: &SubMenu, commands: &Commands) {
    let push = |command: Command| {
        let commands = commands.clone();
        move || commands.borrow_mut().push_back(command)
    };
    bar.on_toggle_menu(push(Command::ToggleMenu));
    let c = commands.clone();
    bar.on_select_desktop(move |i| c.borrow_mut().push_back(Command::SelectDesktop(i)));
    menu.on_hover_submenu(push(Command::OpenSubMenu));
    menu.on_hover_other(push(Command::CloseSubMenu));
    let c = commands.clone();
    menu.on_activated(move |_| c.borrow_mut().push_back(Command::CloseMenus));
    let c = commands.clone();
    submenu.on_activated(move |i| {
        let mut c = c.borrow_mut();
        c.push_back(Command::SelectDesktop(i));
        c.push_back(Command::CloseMenus);
    });
}

/// Mode automatique pour les mesures, au même rythme que le prototype iced : par pas de
/// 250 ms, 4 s de repos, puis dix cycles de 2 s où le menu s'ouvre, le sous-menu s'ouvre
/// 750 ms plus tard, et tout se ferme à 1,25 s.
fn start_auto(commands: Commands) -> Timer {
    const IDLE_STEPS: u32 = 16;
    const CYCLES: u32 = 10;
    let timer = Timer::default();
    let mut step = 0u32;
    let mut remaining = CYCLES;
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        step += 1;
        if step < IDLE_STEPS {
            return;
        }
        let mut c = commands.borrow_mut();
        match (step - IDLE_STEPS) % 8 {
            0 if remaining == 0 => c.push_back(Command::Exit),
            0 => {
                remaining -= 1;
                c.push_back(Command::ToggleMenu);
            }
            3 => c.push_back(Command::OpenSubMenu),
            5 => c.push_back(Command::CloseMenus),
            _ => {}
        }
    });
    timer
}

const DAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Heure et date au format de la planche : « 21:46 » et « mar. 24 nov. ».
fn now_strings() -> (String, String) {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let clock = format!("{:02}:{:02}", now.hour(), now.minute());
    let day = DAYS[now.weekday().num_days_from_monday() as usize];
    let month = MONTHS[now.month0() as usize];
    (clock, format!("{day} {} {month}", now.day()))
}
