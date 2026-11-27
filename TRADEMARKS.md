# Name, logo and mascot

The code of SherlockOS is free software under the GNU General Public License, version 3 or later
(see `LICENSE` and `NOTICE`). The identity of the project is not: **the name, the logo and the
mascot are outside the license of the code, all rights reserved.**

## What is reserved

| Element | Where |
| --- | --- |
| The name "SherlockOS", and any name or logo close enough to be confused with it | Everywhere it is shown to a user as the name of the system |
| The logo: the green ring with its cream dot, in every color and size | `desktop/sherlock-ui/logo/` |
| Sherlock, the mascot: the character, its 3D model, its animations and seasonal variants | Model in the separate repository `dodolereporter/sherlock`, fetched by `just mascot`, never stored here |
| Pictures of Sherlock | `docs/design/images/` (one of them, `sherlock-neutre.png`, is built into `sherlock-3d` as the still fallback), the boards of `docs/design/` |
| The Sherlock application icon | `desktop/sherlock-ui/icons/apps/sherlock.svg` and its exports in `desktop/sherlock-ui/icons/hicolor/` |
| The wallpapers and the lock screen pictures, which show the logo or the mascot | `assets/wallpapers/*.jpg`, `assets/lock/*.jpg` |

## What this means

- None of these elements may be reused, copied, modified or redistributed outside this
  repository without the prior written agreement of the project's author.
- **Forks must change the name and the logo, and remove the mascot** (the files listed above,
  and the `just mascot` recipe that fetches the model). The GPL lets you take the code; it does
  not let you present your fork as SherlockOS.
- Plain factual references are fine: "based on SherlockOS", "a fork of SherlockOS", a link to
  this repository, or the name in a news article or a review.
- Building and running this repository unchanged, for yourself, to try it or to contribute to
  it, is fine.

## Contributions

A contribution that changes one of these elements (a new wallpaper, a new picture of Sherlock,
a change to the logo) is accepted only if its author agrees that it joins the reserved identity
of the project. Say so in the pull request.

## Contact

To ask for an agreement, open an issue on GitHub. Do not post private details in it: the
discussion moves elsewhere once the request is understood.
