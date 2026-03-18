# Configuration

Ce document couvre :

- les variables d'environnement reconnues
- les valeurs compilees par defaut
- les differences entre mode standalone, dev et Traefik
- les points de securite a verifier avant production

## Variables d'environnement

### Variables runtime de l'application

| Variable | Requise | Defaut | Description |
| --- | --- | --- | --- |
| `SECRET_KEY` | Oui en pratique | placeholder local | Cle secrete chargee dans la config. Elle est actuellement validee et stockee par l'app ; definissez toujours une valeur unique en local, staging et prod. |
| `SECURE_COOKIES` | Non | `false` | Si `true`, les cookies de session et CSRF sont emis avec l'attribut `Secure`. A activer derriere HTTPS. |
| `DMXFORGE_STATIC_DIR` | Non | chemin compile vers `crates/dmxforge/static` | Override avance du dossier static servi par Axum. Principalement utile pour packaging custom. |

### Variables utilisees seulement par le compose Traefik

| Variable | Requise | Description |
| --- | --- | --- |
| `PUBLIC_DOMAIN` | Oui avec `docker-compose.traefik.yml` | Domaine public utilise dans la regle Traefik `Host(...)` |

## Valeurs par defaut compilees

Ces valeurs sont definies dans `crates/dmxforge/src/config.rs`.

| Parametre | Valeur |
| --- | --- |
| Nom applicatif | `DmxForge` |
| Bind address | `0.0.0.0` |
| Port | `3000` |
| Base SQLite | `sqlite://data/dmxforge.db` |
| Pool SQLite | `5` connexions |
| Cookie de session | `dmxforge_session` |
| TTL session | `24` heures |
| Payload max | `512` KiB |

## Exemple `.env`

Mode standalone / local :

```dotenv
SECRET_KEY=replace-with-a-random-32-char-secret
SECURE_COOKIES=false
```

Mode Traefik :

```dotenv
SECRET_KEY=replace-with-a-random-32-char-secret
SECURE_COOKIES=true
PUBLIC_DOMAIN=dmxforge.example.com
```

## Resolution de l'URL publique

DmxForge n'utilise plus `PUBLIC_BASE_URL`.

Les URLs webhook affichees dans l'interface sont derivees dynamiquement a partir des headers de la requete :

- `X-Forwarded-Proto`
- `X-Forwarded-Host`
- `Forwarded`
- `Host`

En l'absence totale de headers, l'app retombe sur `http://127.0.0.1:3000`.

## Stockage

### SQLite

En local et dans l'image Docker, la base est stockee ici :

```text
data/dmxforge.db
```

Dans le compose standalone ou Traefik, ce chemin est persiste via le volume Docker :

```text
/app/data
```

### Static assets

Par defaut, l'app sert :

```text
crates/dmxforge/static
```

Dans l'image Docker, ce dossier est copie vers :

```text
/app/static
```

## Cookies et session

Le comportement depend de `SECURE_COOKIES` :

- `false`
  - pratique en HTTP local
  - non recommande en production
- `true`
  - requis derriere HTTPS
  - a utiliser avec Traefik, Caddy ou tout reverse proxy TLS

## Notes de securite

### `SECRET_KEY`

Etat actuel du code :

- la valeur est validee au demarrage
- elle est chargee dans `AppConfig`
- elle n'est pas encore branchee sur une signature de cookie ou un chiffrement applicatif complet

Conseil :

- gardez quand meme une valeur unique et longue
- ne commitez jamais une vraie cle dans le depot

### `SECURE_COOKIES`

Regle simple :

- local HTTP : `false`
- staging/prod HTTPS : `true`

### Donnees runtime sensibles

Le repo peut etre propre alors que votre workspace local ne l'est pas. La base SQLite peut contenir :

- emails utilisateurs
- sessions
- URLs webhook Discord
- payloads et headers de livraisons

Avant une diffusion ou un backup, verifiez le contenu de `data/dmxforge.db`.

## Compose disponibles

### `docker-compose.yml`

Usage :

- execution standalone simple
- expose `3000:3000`
- persiste `/app/data`

### `docker-compose.dev.yml`

Usage :

- developpement en conteneur
- monte le repo local dans `/app`
- lance `cargo run -p dmxforge`

### `docker-compose.traefik.yml`

Usage :

- reverse proxy HTTPS avec Traefik
- volume persistant SQLite
- labels Traefik pour TLS et middlewares

## Verification rapide

### Healthcheck

```bash
curl http://localhost:3000/health
```

### Migrations seulement

```bash
cargo run -p dmxforge -- --migrate-only
```
