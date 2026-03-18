# Changelog

Ce projet suit une convention proche de Keep a Changelog.

## [Unreleased]

- Galerie de screenshots versionnee
- Guide de profil utilisateur et procedures admin avancees
- Documentation d'exploitation plus detaillee autour de la retention SQLite

## [0.1.0] - 2026-03-18

### Added

- Shell d'administration SSR avec dashboard, sources, destinations, templates, regles, livraisons et utilisateurs
- Setup initial, login, logout, sessions SQLite et protection CSRF
- Roles `superadmin`, `admin`, `editor`, `viewer`
- Permissions deleguees par utilisateur et sous-utilisateurs lies a un parent
- Ownership des ressources avec filtrage serveur par utilisateur
- CRUD des sources webhook, destinations Discord, templates de message et regles de routage
- Historique des livraisons avec detail et replay manuel
- Preview live des templates Discord
- Support webhook GitHub, GitLab et Gitea / Forgejo
- Docker standalone, Docker dev et Docker + Traefik
- Documentation de base : README, configuration, templates, payloads, deploiement Traefik, contribution

### Changed

- Les URLs webhook affichees dans l'UI sont maintenant derivees des headers de requete
- La page `/settings` a ete retiree de l'UI et redirige vers le dashboard
- Les modales de formulaire gerent maintenant correctement la hauteur d'ecran et le scroll
- Les tables et previews UI ont ete compactees et reequilibrees

### Security

- Validation stricte des URLs webhook Discord en HTTPS sur domaines officiels
- CSRF ajoute sur les formulaires anonymes critiques (`/setup`, `/login`)
- Rate limiting de login base sur l'adresse socket reelle
- Permissions appliquees cote serveur sur les ressources, livraisons et utilisateurs

### Performance

- Nettoyage de sessions retire du chemin chaud de navigation
- Touch session limitee a un rafraichissement periodique
- Calculs inutiles de preview supprimes sur plusieurs vues SSR

### Tests

- Suite unitaire Rust verte sur les modules auth, config, Discord et webhook
