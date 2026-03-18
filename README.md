# DmxForge

DmxForge est une application web Rust self-hosted pour recevoir des webhooks Git, les normaliser, appliquer des regles de routage et publier des messages formates sur Discord.

Le projet est construit autour d'un shell d'administration SSR, d'un stockage SQLite, d'un moteur de templates Minijinja et d'un pipeline webhook simple a deployer en Docker.

## Ce que fait DmxForge

1. Recoit un webhook entrant sur `POST /webhooks/{provider}/{token}`.
2. Verifie eventuellement la signature ou le token du provider.
3. Normalise le payload dans un modele commun.
4. Filtre les sources et les regles de routage.
5. Rend un message Discord a partir d'un template Minijinja.
6. Envoie le resultat vers une destination Discord.
7. Journalise la livraison, les erreurs et les tentatives d'envoi.

## Fonctionnalites principales

- UI admin SSR avec Axum + Askama
- Authentification avec setup initial, login, logout, sessions SQLite et CSRF
- Roles `superadmin`, `admin`, `editor`, `viewer`
- Permissions deleguees par utilisateur et sous-utilisateurs lies a un parent
- CRUD complet pour les sources, destinations, templates, regles et utilisateurs
- Support webhook pour GitHub, GitLab et Gitea / Forgejo
- Historique des livraisons et rejeu manuel d'une livraison
- Preview live des templates Discord
- Docker standalone, Docker dev et Docker + Traefik

## Apercu UI

Le depot ne versionne pas encore de galerie d'images binaire. A la place, la navigation et le comportement de chaque ecran sont decrits dans [docs/ui-overview.md](docs/ui-overview.md).

Pages actuellement exposees :

| Route | Usage |
| --- | --- |
| `/dashboard` | Vue operationnelle, activite 7 jours, top repos, top evenements, dernieres livraisons |
| `/sources` | Gestion des entrees webhook Git |
| `/destinations` | Gestion des webhooks Discord de sortie |
| `/templates` | Gestion des templates Minijinja et preview Discord |
| `/rules` | Routage source -> template -> destination avec filtres |
| `/deliveries` | Historique des livraisons, detail et replay |
| `/users` | Gestion des utilisateurs, sous-utilisateurs et permissions |
| `/login` | Connexion |
| `/setup` | Creation du premier compte si aucun admin n'existe |

## Stack technique

- Rust 1.88
- Axum
- Tokio
- Askama
- SQLx + SQLite
- Minijinja
- Reqwest + Rustls
- Tower HTTP
- Tracing
- Docker / Traefik

## Architecture

### Vue d'ensemble

- `crates/dmxforge/src/web/`
  - UI SSR, auth web, routes d'administration, preview API, healthcheck
- `crates/dmxforge/src/webhook/`
  - Endpoint webhook, verification provider, normalisation, rendu Discord, routage
- `crates/dmxforge/src/discord/`
  - Moteur Minijinja, preview, validation des URLs Discord
- `crates/dmxforge/src/db/`
  - Acces SQLite, stats, users, deliveries, ressources
- `crates/dmxforge/templates/`
  - Templates Askama de l'interface
- `crates/dmxforge/static/`
  - CSS et JS du shell admin

### Flux webhook

```text
Provider Git
  -> /webhooks/{provider}/{token}
  -> verification signature / token
  -> normalisation UnifiedEvent
  -> filtrage source
  -> matching regles actives
  -> rendu template Discord
  -> POST webhook Discord
  -> journalisation livraison + tentative Discord
```

## Providers et evenements supportes

| Provider | Header type evenement | Verification | Evenements normalises |
| --- | --- | --- | --- |
| GitHub | `X-GitHub-Event` | `X-Hub-Signature-256` prefere, `X-Hub-Signature` en fallback | `push`, `pull_request`, `issues`, `release` |
| GitLab | `X-Gitlab-Event` | `X-Gitlab-Token` | `push`, `tag_push`, `merge_request`, `pipeline`, `release` |
| Gitea / Forgejo | `X-Gitea-Event-Type` | `X-Gitea-Signature` ou `X-Gogs-Signature` | `push`, `pull_request`, `issues`, `release` |

Notes :

- Les payloads sont normalises dans un modele commun pour le rendu.
- Le champ brut provider-specifique reste disponible via `metadata` dans les templates.
- Les URLs webhook affichees dans l'UI sont derivees des headers de requete entrants, pas d'une variable `PUBLIC_BASE_URL`.

## Demarrage rapide

### Option 1 - Docker standalone

1. Copier le fichier d'exemple :

```bash
cp .env.example .env
```

2. Editer au minimum `SECRET_KEY` dans `.env`.

3. Lancer l'application :

```bash
docker compose up --build -d
```

4. Ouvrir :

```text
http://localhost:3000
```

5. Si aucun compte n'existe, DmxForge redirige automatiquement vers `/setup`.

### Option 2 - Docker + Traefik

Le guide detaille est dans [docs/deployment-traefik.md](docs/deployment-traefik.md).

Resume minimal :

```bash
cp .env.example .env
# definir SECRET_KEY, SECURE_COOKIES=true et PUBLIC_DOMAIN
docker compose -f docker-compose.traefik.yml up --build -d
```

### Option 3 - Developpement local Rust

Prerequis :

- Rust 1.88
- SQLite embarque via SQLx

Lancer :

```bash
cargo run -p dmxforge
```

Ou avec le Makefile :

```bash
make dev
```

Application disponible sur :

```text
http://localhost:3000
```

## Configuration

La configuration complete est documentee dans [docs/configuration.md](docs/configuration.md).

### Variables d'environnement runtime

| Variable | Scope | Defaut | Usage |
| --- | --- | --- | --- |
| `SECRET_KEY` | App | placeholder local | Cle secrete exigee par la config, a rendre unique par environnement |
| `SECURE_COOKIES` | App | `false` | Force les cookies en mode `Secure` derriere HTTPS |
| `DMXFORGE_STATIC_DIR` | Avance | chemin compile | Surcharge le dossier static servi par Axum |
| `PUBLIC_DOMAIN` | Traefik seulement | aucun | Utilise par `docker-compose.traefik.yml` pour la regle d'hote Traefik |

### Valeurs compilees / par defaut

- bind : `0.0.0.0`
- port : `3000`
- base SQLite : `sqlite://data/dmxforge.db`
- pool SQLite : `5`
- cookie de session : `dmxforge_session`
- TTL session : `24h`
- limite payload : `512 KiB`

## Roles et permissions

| Role | Capacites principales |
| --- | --- |
| `superadmin` | Acces total, gestion de tous les utilisateurs et de toutes les ressources |
| `admin` | Gestion de ses sous-utilisateurs, lecture/ecriture sur les ressources, replay des livraisons |
| `editor` | Lecture/ecriture sur les ressources visibles, pas de gestion utilisateurs |
| `viewer` | Lecture seule sur les pages visibles |

Le systeme implemente aussi des permissions deleguees plus fines :

- lecture / ecriture des sources
- lecture / ecriture des destinations
- lecture / ecriture des templates
- lecture / ecriture des regles
- lecture des livraisons
- replay des livraisons
- lecture / ecriture des utilisateurs
- creation de sous-utilisateurs

Le scope est applique cote serveur :

- un utilisateur ne voit que son sous-arbre autorise
- un parent ne peut pas deleguer plus que ses propres permissions
- les ressources sont rattachees a un `user_id`

## Templates Discord

Le detail complet est dans [docs/templates.md](docs/templates.md).

Points importants :

- moteur : Minijinja
- preview UI : `POST /api/preview`
- sample payload de preview inclus dans l'app
- contexte runtime commun pour tous les providers normalises
- `metadata` expose le payload brut provider-specifique

Exemple simple :

```jinja
{{ actor.name }} pushed {{ commit_count }} commits to {{ repository.full_name }}
{% if branch %}on {{ branch }}{% endif %}
```

## Endpoints utiles

| Endpoint | Type | Description |
| --- | --- | --- |
| `/health` | public JSON | Etat de l'instance et ping SQLite |
| `/api/preview` | JSON authentifie | Preview d'un template Minijinja |
| `/webhooks/{provider}/{token}` | public POST | Entree webhook provider |
| `/deliveries/{id}` | UI | Detail d'une livraison |
| `/deliveries/{id}/replay` | POST | Rejoue une livraison existante |

Exemple healthcheck :

```bash
curl http://localhost:3000/health
```

## Docker disponibles

| Fichier | Usage |
| --- | --- |
| `docker-compose.yml` | Execution standalone simple |
| `docker-compose.dev.yml` | Developpement dans le conteneur avec `cargo run` |
| `docker-compose.traefik.yml` | Deploiement avec reverse proxy Traefik |

## Arborescence utile

```text
.
├── crates/dmxforge/
│   ├── migrations/
│   ├── src/
│   │   ├── auth/
│   │   ├── db/
│   │   ├── discord/
│   │   ├── web/
│   │   └── webhook/
│   ├── static/
│   └── templates/
├── deploy/
├── docker-compose.yml
├── docker-compose.dev.yml
├── docker-compose.traefik.yml
├── CHANGELOG.md
├── Makefile
└── TODO.md
```

## Documentation detaillee

- [docs/ui-overview.md](docs/ui-overview.md)
- [docs/configuration.md](docs/configuration.md)
- [docs/templates.md](docs/templates.md)
- [docs/payload-examples.md](docs/payload-examples.md)
- [docs/deployment-traefik.md](docs/deployment-traefik.md)
- [docs/contributing.md](docs/contributing.md)
- [CHANGELOG.md](CHANGELOG.md)

## Contribution

Le guide contribution est dans [docs/contributing.md](docs/contributing.md).

Commandes les plus utiles :

```bash
make check
make test
make fmt
make lint
make migrate
```

## Limitations actuelles

- pas encore de retries Discord automatiques
- pas de destination Slack / Teams / ntfy
- pas de REST admin separee
- pas de galerie de screenshots versionnee dans le depot

## Licence

MIT
