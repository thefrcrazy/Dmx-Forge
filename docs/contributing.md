# Contributing

Ce document decrit comment travailler proprement sur DmxForge.

## Prerequis

- Rust 1.88
- Docker si vous utilisez les compose
- connaissance minimale de Axum, Askama, SQLx et SQLite

## Workflow local

### Lancer l'application

```bash
make dev
```

Equivalent :

```bash
cargo run -p dmxforge
```

### Verifications standard

```bash
make check
make test
make fmt
make lint
```

### Migrations seulement

```bash
make migrate
```

## Structure du code

| Zone | Role |
| --- | --- |
| `src/web/` | routes SSR, auth web, preview, health |
| `src/webhook/` | reception webhook, verification, normalisation, envoi Discord |
| `src/discord/` | moteur Minijinja et validation Discord |
| `src/db/` | acces SQLite |
| `templates/` | templates Askama |
| `static/` | CSS et JS du shell admin |
| `migrations/` | schema SQL versionne |

## Recommandations de contribution

### 1. Garder le scope clair

Une PR ou un lot de modifications doit idealement viser un seul axe :

- bugfix
- feature backend
- feature UI
- migrations
- documentation

### 2. Verifier l'impact sur les permissions

Le projet applique maintenant :

- des roles
- des permissions deleguees
- un scope parent -> sous-utilisateurs
- un ownership des ressources

Toute nouvelle route admin doit etre verifiee sous cet angle.

### 3. Si vous touchez la base

- ajouter une migration dans `crates/dmxforge/migrations/`
- garder la migration idempotente autant que possible
- verifier les cas d'upgrade depuis une base existante

### 4. Si vous touchez les templates Discord

- tester la preview UI
- verifier au moins un cas `push`
- verifier au moins un cas PR / MR ou `release`
- traiter les champs optionnels proprement

### 5. Si vous touchez les providers webhook

- verifier les headers attendus
- verifier la normalisation dans `UnifiedEvent`
- ajouter ou ajuster les tests unitaires

## Style de code

- formater avec `cargo fmt`
- garder `cargo clippy -- -D warnings` propre
- preferer des changements SSR simples et lisibles
- eviter de disperser la logique metier dans le JS si elle peut rester cote serveur

## Tests

Avant de proposer une modification :

```bash
cargo test --workspace
```

Minimum attendu :

- pas de regression sur les tests `auth`
- pas de regression sur les tests `discord`
- pas de regression sur les tests `webhook`

## Documentation

Si vous changez un comportement utilisateur ou operatoire, mettez a jour au moins un des fichiers suivants :

- `README.md`
- `CHANGELOG.md`
- `docs/configuration.md`
- `docs/templates.md`
- `docs/payload-examples.md`
- `docs/deployment-traefik.md`
- `docs/ui-overview.md`

## Checklist de contribution

- code compile
- tests verts
- migration ajoutee si schema modifie
- UI revue sur desktop et mobile si necessaire
- docs mises a jour
- pas de secrets reels commits
