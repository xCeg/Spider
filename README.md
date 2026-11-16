# Spider

**Discord Rich Presence, sans token.** Spider affiche jusqu'à trois cartes d'activité sur ton
profil Discord (« Joue à… », « Regarde… », « Écoute… », en stream, avec images, boutons et
chronomètre). Il ne demande ni bot, ni token, ni mot de passe : il parle au Discord installé sur
ton PC, par le même canal que les jeux.

## Installer

1. Double-clique **`build.bat`**.
   - Il vérifie que Rust est installé (sinon il propose de l'installer avec winget) ;
   - il compile Spider en mode release (une à deux minutes la première fois) ;
   - il pose **`Spider.exe`** à côté de lui et propose de le lancer tout de suite.
2. Ensuite, double-clique simplement **`Spider.exe`**.

## Ce que fait la fenêtre

- **L'ouverture** : le logo SPIDER arrive en balayage, avec un dégradé de couleurs et un
  reflet lumineux. Une touche passe l'animation.
- **Premier lancement** : un assistant pose les questions une par une, avec l'aperçu de la
  carte qui se met à jour pendant que tu tapes :
  - un **modèle** pour partir vite (Spotify, YouTube, Twitch, Crunchyroll, League of Legends)
    ou ta propre application ;
  - l'**ID de l'application**, vérifié directement chez Discord (« Application trouvée : … ») ;
  - le type (Joue à, Streame, Écoute, Regarde, Participe à), le titre, les deux lignes, les
    images, le badge console, les boutons, le chronomètre.
- **Le menu** : lancer la présence, configurer, retirer les cartes, diagnostic, thème,
  relancer Discord, quitter. Une ligne d'état dit en direct si Discord est ouvert, avec quel
  compte, si le port de debug est ouvert et combien de cartes sont prêtes.
- **Le tableau de bord** : l'état (EN LIGNE, ÉCHEC, EN PAUSE), le transport utilisé, le
  compte, depuis quand, l'aperçu de chaque carte et le journal.
  `P` pause, `R` renvoyer, `E` modifier, `X` arrêter, `Échap` revenir au menu (la présence
  continue). Si une carte demande le port de debug (« Streame », logo de console) alors qu'il
  est fermé, le tableau de bord le dit et `D` relance Discord avec lui.
- **Le diagnostic** : les Discord installés et ouverts (Discord, PTB, Canary) avec leur compte,
  le port de debug, la config et ses avertissements, les applications. `T` publie la vraie
  carte pendant 8 secondes puis la retire : la preuve que tout marche.
- **Six thèmes** : Venom, Blood, Ocean, Toxic, Sunset, Ghost.

Quitter Spider (ou fermer sa fenêtre) retire les cartes de ton profil.

Raccourcis partout : flèches pour choisir, `Entrée` pour valider, `Échap` pour revenir,
chiffres pour aller vite, `Ctrl+V` pour coller, `Ctrl+U` pour vider un champ, `Ctrl+S` pour
enregistrer la configuration.

## Plusieurs Discord ouverts

Si Discord, Discord PTB ou Canary tournent en même temps (souvent avec des comptes
différents), choisis dans **Configurer → Client Discord** celui qui doit porter la carte.
En **Auto**, Spider prend le premier ouvert et y reste.

## Le fichier `spider.json`

L'assistant l'écrit pour toi, à côté de `Spider.exe`. Tu peux aussi l'éditer à la main :
Spider le relit à chaque sauvegarde et la carte suit dans la seconde.

```json
{
  "enabled": true,
  "transport": "auto",
  "client": "auto",
  "refresh_seconds": 15,
  "retry_seconds": 5,
  "theme": "venom",
  "activities": [
    {
      "app_id": 123456789012345678,
      "name": "",
      "details": "En train de coder",
      "state": "Projet Spider",
      "image": "https://exemple.com/logo.png",
      "image_text": "Spider",
      "small_image": "",
      "small_text": "",
      "status": "playing",
      "platform": "",
      "stream_url": "",
      "buttons": [{ "label": "Mon site", "url": "https://exemple.com" }],
      "timer": true
    }
  ]
}
```

| champ | rôle |
| --- | --- |
| `enabled` | `false` retire les cartes et attend (le fichier reste surveillé). |
| `transport` | `auto` (port de debug s'il est ouvert, sinon le pipe), `cdp` (port de debug seul), `rpc` (pipe seul). |
| `client` | `auto`, `stable`, `ptb` ou `canary` : le Discord (et donc le compte) qui porte les cartes. |
| `refresh_seconds` | renvoi d'une carte inchangée (2 au minimum). |
| `retry_seconds` | premier nouvel essai après un échec ; il s'allonge ensuite jusqu'à une minute. |
| `theme` | les couleurs de la fenêtre. |
| `app_id` | l'application Discord qui porte la carte (son nom est le titre). **Obligatoire.** |
| `name` | remplace le titre (sûr par le port de debug) ; vide = nom de l'application. |
| `details` / `state` | les deux lignes sous le titre (128 caractères au plus). |
| `image` / `small_image` | une clé d'image de l'application (Art Assets) ou un lien `https://`. |
| `image_signer` | l'application qui signe les images `https://` pour le port de debug ; `0` = `app_id`. |
| `status` | `playing`, `streaming`, `listening`, `watching`, `competing`. |
| `platform` | `xbox`, `ps5`, `meta_quest` : une pastille de console sur l'image (partout, si `small_image` est vide) et le logo à côté du nom (port de debug). |
| `stream_url` | le lien Twitch ou YouTube d'une carte `streaming`. |
| `buttons` | deux boutons au maximum, `label` (32 caractères) + lien `http(s)://`. |
| `timer` | affiche le temps écoulé. |
| `slot` | 1 à 3 ; laissé vide, les cartes prennent 1, 2, 3 dans l'ordre. |

## Les deux transports

| | pipe local (`rpc`) | port de debug (`cdp`) |
| --- | --- | --- |
| prérequis | rien : Discord ouvert suffit | Discord relancé avec `--remote-debugging-port=9222` (menu « Relancer Discord ») |
| « Streame » | non (s'affiche en « Joue à ») | oui |
| badge console | non | oui |
| titre libre (`name`) | pas garanti (souvent le nom de l'application) | oui |
| boutons, images, chronomètre | oui | oui |

`auto` garde le transport qui a marché, pour que la carte ne saute pas d'un style à l'autre.
**Prudence :** tant que Discord tourne avec le port de debug, n'importe quel programme de ton
PC peut le piloter. Ne garde ce mode que si tu en as besoin.

## En ligne de commande

```bat
Spider.exe                      REM la fenêtre
Spider.exe run                  REM publie et suit spider.json, sans fenêtre
Spider.exe once                 REM publie une fois puis quitte
Spider.exe clear                REM retire les cartes
Spider.exe doctor               REM le diagnostic en texte
Spider.exe init                 REM écrit un spider.json d'exemple
Spider.exe restart              REM relance Discord avec le port de debug
Spider.exe --config autre.json
```

Le journal s'écrit à côté du fichier (`spider.log`), avec l'heure.

## Dépannage

- **« Discord ne répond plus »** : Discord n'accepte que deux connexions rapprochées par
  application, puis ignore les suivantes pendant environ une minute. Spider garde sa connexion
  ouverte et espace ses essais (5 s, 10 s, 20 s… jusqu'à une minute) : attends, la carte revient
  seule.
- **« ID d'application inconnu »** : recopie l'Application ID de ton application sur
  discord.com/developers/applications.
- **La carte va sur le mauvais compte** : choisis le bon dans Configurer → Client Discord.
- **« Streame » s'affiche en « Joue à »** : le pipe ne connaît pas le stream ; relance Discord
  avec le port de debug (menu) ou passe le transport sur `cdp`.
- **Une carte reste après la fermeture** : `Spider.exe clear`.

## Le code

- `src/engine.rs` : les deux transports (pipe et port de debug), le choix du client Discord,
  les limites de Discord.
- `src/config.rs` : le format de `spider.json`.
- `src/runner.rs` : la boucle qui garde les cartes en ligne.
- `src/doctor.rs` : le diagnostic.
- `src/ui/` : la fenêtre (logo et animation, menu, assistant, tableau de bord, thèmes).
- `tools/snapper/` : capture la vraie fenêtre dans une console virtuelle, en HTML (pour les
  développeurs).

Tests : `cargo test`. Avec Node installé, `tests/cdp.rs` teste aussi le port de debug de bout
en bout contre un faux Discord (`tests/fake_discord.mjs`) qui exécute le vrai script envoyé à
Discord.
