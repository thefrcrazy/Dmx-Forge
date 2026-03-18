# DmxForge — TODO & Roadmap

> Application web self-hosted en Rust pour router et formatter des webhooks Git vers Discord.
> Design inspiré de Mistral AI — UI/UX premium, SSR via Askama, Docker + Traefik en prod.

---

## 🗂️ Légende

- `[ ]` À faire
- `[~]` En cours
- `[x]` Terminé
- `[!]` Bloquant / Prioritaire

---

## 🏗️ Phase 0 — Fondations & Architecture

### Stack technique

- [x] Initialiser le workspace Cargo (Axum + Tokio)
- [x] Configurer SQLx + SQLite avec mode WAL et foreign keys
- [x] Mettre en place les migrations SQLx (versionnées)
- [x] Intégrer Askama pour le rendu SSR des templates HTML
- [x] Configurer `tracing` + `tracing-subscriber` pour les logs structurés
- [x] Ajouter `tower-http` : TraceLayer, CompressionLayer, Request ID
- [x] Configurer `reqwest` avec Rustls (pas d'OpenSSL)
- [x] Configurer Minijinja pour le rendu des messages Discord
- [x] Définir la structure du projet (modules : `web`, `webhook`, `discord`, `db`, `config`, `auth`)

### Configuration runtime

- [x] Support des variables d'environnement :
  - [x] `SECURE_COOKIES`
  - [x] `SECRET_KEY`
- [x] Chargement de config depuis `.env` en dev local
- [x] Valider la config au démarrage avec messages d'erreur clairs

### Base de données — schéma

- [x] Table `users`
- [x] Table `sessions`
- [x] Table `sources`
- [x] Table `discord_destinations`
- [x] Table `message_templates`
- [x] Table `routing_rules`
- [x] Table `webhook_deliveries`
- [x] Table `discord_messages`
- [x] Table `app_settings`
- [x] Table `audit_logs`
- [x] Table `user_shared_configs` _(nouveau)_
- [x] Table `sub_users` / lien parent-enfant dans `users` _(nouveau)_

---

## 🔐 Phase 1 — Authentification & Gestion des utilisateurs

### Setup initial

- [x] Page `/setup` — création du premier compte administrateur
- [x] Redirection automatique vers `/setup` si aucun admin n'existe
- [x] Validation : mot de passe minimum 12 caractères + confirmation
- [x] Hash Argon2 des mots de passe
- [x] Seed des paramètres système par défaut au premier démarrage

### Connexion / Déconnexion

- [x] Page `/login` (GET + POST)
- [x] Création de session côté serveur (SQLite)
- [x] Cookie de session : `HttpOnly`, `SameSite=Lax`, `Secure` configurable, TTL configurable
- [x] Token CSRF par session, vérification sur tous les formulaires mutables
- [x] Route `POST /logout` — invalidation de la session
- [x] Limite de tentatives de connexion (rate limiting par IP)

### Système de rôles & permissions

- [ ] Rôles de base : `superadmin`, `admin`, `editor`, `viewer`
- [ ] Rôle `superadmin` : accès total, gestion de tous les utilisateurs
- [ ] Rôle `admin` : gestion des utilisateurs qu'il a créés + toutes les ressources
- [ ] Rôle `editor` : création/modification des ressources, pas de gestion d'utilisateurs
- [ ] Rôle `viewer` : lecture seule sur toutes les pages
- [ ] Middleware de vérification de rôle sur chaque route admin
- [ ] Permissions granulaires optionnelles par ressource _(avancé)_

### Gestion des utilisateurs (CRUD complet)

> Un utilisateur ne peut gérer QUE les utilisateurs qu'il a lui-même créés (sauf superadmin).

- [x] Page `/users` — liste des utilisateurs (filtrée selon le rôle)
  - [x] Affichage : nom, email, rôle, statut (actif/inactif), date de création, créé par
  - [x] Barre de recherche + filtres par rôle / statut
- [ ] Création d'un utilisateur via modal
  - [ ] Champs : username, email, mot de passe, rôle, statut actif
  - [ ] L'utilisateur créateur est enregistré comme `parent_user_id`
- [ ] Édition d'un utilisateur via modal
  - [ ] Modification : username, email, rôle, statut
  - [ ] Réinitialisation du mot de passe (génération ou saisie manuelle)
  - [ ] Un admin ne peut pas élever un sous-utilisateur au-dessus de son propre rôle
- [ ] Suppression d'un utilisateur (avec confirmation modale)
  - [ ] Cascade ou réassignation des ressources orphelines
- [ ] Désactivation/réactivation d'un compte (soft delete)
- [ ] Vue "arbre" des sous-utilisateurs (qui a créé qui)
- [ ] Page de profil `/profile` — modification de ses propres infos et mot de passe

### Création de sous-utilisateurs avec permissions déléguées

- [ ] Un utilisateur `admin` peut créer des sous-utilisateurs
- [ ] Les sous-utilisateurs sont liés à leur créateur (`parent_user_id`)
- [ ] Un sous-utilisateur ne peut accéder qu'aux ressources de son parent OU aux siennes
- [ ] Le parent peut révoquer ou modifier les permissions de ses sous-utilisateurs à tout moment
- [ ] Un sous-utilisateur ne peut pas créer d'autres sous-utilisateurs (sauf si `admin`)
- [ ] Tableau de bord parent : voir l'activité de ses sous-utilisateurs

---

## 🎨 Phase 2 — Design Frontend (Mistral AI Style)

### Design system & tokens

- [x] Police : Inter (Google Fonts) — titres bold, corps regular
- [x] Palette de couleurs :
  - [x] Fond : `#0A0A0F` (très sombre, quasi-noir)
  - [x] Surface : `#111118` + `#1A1A24`
  - [x] Accent principal : `#FF7000` (orange Mistral) ou `#7C3AED` (violet)
  - [x] Texte : `#F5F5F7` / `#9CA3AF`
  - [x] Succès : `#10B981`, Erreur : `#EF4444`, Warning : `#F59E0B`
  - [x] Bordures subtiles : `rgba(255,255,255,0.08)`
- [x] Variables CSS custom properties globales
- [x] Composants de base : boutons, badges, inputs, modales, tooltips
- [x] Animations : transitions `200ms ease`, hover effects, focus rings
- [x] Toggle dark/light mode avec persistance `localStorage` + respect système

### Layout principal (shell d'admin)

- [x] Sidebar navigation fixe (collapsible sur petits écrans)
  - [x] Logo + nom de l'app en haut
  - [x] Liens : Dashboard, Sources, Discord, Templates, Routes, Livraisons, Utilisateurs
  - [ ] Avatar + nom utilisateur + rôle en bas de sidebar
  - [ ] Indicateur de statut de l'instance (health)
- [ ] Header avec breadcrumb + actions contextuelles
- [x] Zone de contenu principale avec padding adaptatif
- [x] Toasts de notification (succès/erreur) animés
- [x] Responsive : sidebar repliée sur mobile, menu hamburger

### Composants UI avancés

- [x] Modal générique avec animation d'ouverture/fermeture fluide
  - [x] Auto-ouverture via `?edit=...` dans l'URL
  - [x] Fermeture au clic extérieur + touche Escape
- [~] Table de données avec :
  - [ ] Tri par colonne
  - [ ] Pagination ou infinite scroll
  - [x] Ligne vide illustrée si pas de données
  - [x] Actions inline (éditer, supprimer, activer/désactiver)
- [x] Badges de statut colorés (actif, inactif, succès, erreur, pending)
- [x] Cards métriques animées avec icônes
- [ ] Breadcrumb dynamique
- [x] Empty states illustrés avec CTA
- [x] Skeleton loading screens
- [x] Confirmation de suppression via modale dédiée

---

## 📊 Phase 3 — Dashboard & Observabilité

- [x] Page `/dashboard` — vue opérationnelle
  - [x] Métriques en cards :
    - [x] Total livraisons
    - [x] Livraisons traitées
    - [x] Livraisons échouées
    - [x] Messages Discord envoyés
  - [x] Graphique d'activité (7 derniers jours)
  - [x] Top 5 dépôts les plus actifs
  - [x] Top 5 types d'événements
  - [x] Liste des 10 dernières livraisons avec statut
  - [ ] Indicateur de santé de l'instance
- [x] Endpoint `GET /health` (JSON public)
- [x] Métriques calculées depuis SQLite au chargement

---

## 🔗 Phase 4 — Sources Webhook

- [x] Page `/sources`
  - [x] Liste des sources avec filtres et recherche
  - [x] Création via modal
  - [x] Édition via modal
  - [x] Suppression avec confirmation
  - [x] Toggle activation/désactivation inline
- [x] Configuration d'une source :
  - [x] Nom, provider (`github`, `gitlab`, `gitea`)
  - [x] Secret webhook (optionnel, affiché masqué)
  - [x] Filtre dépôt, branches autorisées, événements autorisés
  - [x] Activation/désactivation
- [x] Affichage de l'URL webhook complète générée (`/webhooks/{provider}/{token}`)
- [x] Bouton "copier l'URL" avec feedback visuel
- [ ] Bouton "régénérer le token" avec confirmation

---

## 📣 Phase 5 — Destinations Discord

- [x] Page `/destinations`
  - [x] Liste des destinations
  - [x] Création via modal
  - [x] Édition via modal
  - [x] Suppression avec confirmation
  - [x] Toggle activation/désactivation inline
- [x] Configuration :
  - [x] Nom, URL webhook Discord (masquée visuellement)
  - [x] Activation/désactivation
- [x] Validation stricte de l'URL Discord :
  - [x] Hôtes autorisés : `discord.com`, `ptb.discord.com`, `canary.discord.com`
  - [x] Path obligatoire `/api/webhooks/...`
- [ ] Test de connexion Discord depuis l'interface (envoi d'un message de test)

---

## 🖋️ Phase 6 — Templates de Messages Discord

- [x] Page `/templates`
  - [x] Liste des templates
  - [x] Création via modal
  - [x] Édition via modal
  - [x] Suppression avec confirmation
- [x] Seed automatique au démarrage :
  - [x] `Compact Push`
  - [x] `Detailed Activity`
  - [x] `Release Notes`
  - [x] `CI Alert`
- [x] Configuration d'un template :
  - [x] Nom, style de format, template Minijinja
  - [x] Couleur hex de l'embed
  - [x] Username override, avatar override
  - [x] Footer text
  - [x] Toggles : afficher avatar, lien repo, branche, commits, badge statut, timestamp
  - [x] Activation/désactivation
- [~] **Live Preview Discord** :
  - [x] Prévisualisation style Discord dans le modal d'édition
  - [x] Rendu en temps réel à la saisie (debounce 300ms)
  - [x] API `POST /api/preview` pour générer le rendu côté serveur
  - [x] Données fictives pré-remplies pour la preview
- [ ] Affichage du body template brut (coloriée syntaxe Jinja2)

---

## 🔀 Phase 7 — Règles de Routage

- [x] Page `/rules`
  - [x] Liste des règles avec badges de filtres actifs
  - [x] Création via modal
  - [x] Édition via modal
  - [x] Suppression avec confirmation
  - [x] Toggle activation/désactivation inline
  - [ ] Réordonnancement drag-and-drop _(avancé)_
- [x] Configuration d'une règle :
  - [x] Source (spécifique ou `*` toutes)
  - [x] Destination Discord
  - [x] Template de message
  - [x] Filtres optionnels :
    - [x] Provider
    - [x] Type d'événement
    - [x] Préfixe de branche
    - [x] Nom exact du dépôt
    - [x] Mot-clé de skip dans les commits
- [x] Visualisation claire des associations source → template → destination
- [x] Test d'une règle depuis l'interface (envoi de payload fictif)

---

## 📦 Phase 8 — Pipeline Webhook (Backend)

### Réception

- [x] Routes publiques :
  - [x] `POST /webhooks/github/{token}`
  - [x] `POST /webhooks/gitlab/{token}`
  - [x] `POST /webhooks/gitea/{token}`
- [x] Recherche de la source par `provider + token`
- [x] Parsing JSON du body
- [x] Sauvegarde immédiate d'une livraison `pending`
- [x] Traitement asynchrone via `tokio::spawn`

### Validation de signature

- [x] GitHub : `X-Hub-Signature-256` (SHA256), fallback `X-Hub-Signature` (SHA1)
- [x] GitLab : `X-Gitlab-Token`
- [x] Gitea/Forgejo : `X-Gitea-Signature` HMAC SHA256
- [x] Si secret vide → vérification désactivée pour cette source

### Normalisation — `UnifiedEvent`

- [x] Modèle commun : provider, event_type, repo, acteur, branche, commits, titre, description, statut, URL, date, metadata raw
- [x] Parsing GitHub : `push`, `pull_request`, `issues`, `release`
- [x] Parsing GitLab : `Push Hook`, `Tag Push Hook`, `Merge Request Hook`, `Pipeline Hook`, `Release Hook`
- [x] Parsing Gitea/Forgejo : `push`, `pull_request`, `issues`, `release`

### Filtres & Matching

- [x] Rejets au niveau source : source désactivée, événement non autorisé, branche non autorisée, dépôt non correspondant
- [x] Matching règles : provider, type d'événement, dépôt (insensible casse), préfixe branche, skip keyword dans commits
- [x] Chargement uniquement des règles/destinations/templates actifs

### Génération & Envoi Discord

- [x] Rendu Minijinja du body template
- [x] Construction embed Discord (titre, URL, couleur, auteur, footer, timestamp, champs)
- [x] Liste des commits limitée aux 5 premiers, IDs raccourcis à 7 chars
- [x] Timeout client `reqwest` à 10 secondes
- [x] Enregistrement `discord_messages` pour chaque tentative

---

## 📜 Phase 9 — Historique des Livraisons

- [x] Page `/deliveries`
  - [x] Liste des 50 dernières livraisons (pagination)
  - [x] Filtres : statut, source, provider, type d'événement, date
  - [x] Barre de recherche (dépôt, hash de commit)
- [x] Page détail `/deliveries/{id}`
  - [x] Métadonnées : source, provider, type, statut, dates, erreur
  - [x] Inspection technique :
    - [x] Headers entrants
    - [ ] Payload brut JSON (coloration syntaxique)
    - [x] Événement normalisé JSON
    - [x] Messages Discord générés
    - [x] Payload exact envoyé à Discord
    - [x] Statut/réponse Discord par message
  - [x] **Bouton "Rejouer la livraison"** — rejouer le pipeline complet depuis le payload brut
- [x] Statut de livraison : `pending`, `processed`, `failed`, `skipped`
- [x] Indicateur clair si livraison OK mais envois Discord partiellement échoués

---

## 🔄 Phase 10 — Partage de Configuration

> Fonctionnalité permettant à un utilisateur de partager sa config (sources, templates, règles) avec un autre.

- [ ] Export d'une configuration : sélection des ressources (sources, templates, règles, destinations) à partager
- [ ] Génération d'un **token de partage** unique (TTL configurable, usage unique optionnel)
- [ ] Page `/import/{token}` — import de la configuration partagée
  - [ ] Prévisualisation des éléments à importer avant validation
  - [ ] Option : dupliquer ou lier les ressources importées
- [ ] Historique des partages effectués par l'utilisateur
- [ ] Révocation d'un token de partage
- [ ] Contrôle : un utilisateur ne peut partager que SES ressources
- [ ] Interface dédiée : bouton "Partager ma config" sur les pages sources/templates/règles

---

## ⚙️ Phase 11 — Paramètres & Audit

### Paramètres instance (`/settings`) — admin only

- [ ] Page `/settings` — lecture runtime + settings persistés + état d'instance
- [ ] Modification : nom d'instance, URL publique, niveau de log, limite payload KB
- [ ] Paramètres de session : TTL, secure cookies
- [ ] Paramètres SMTP (notifications email futures) _(optionnel)_

### Audit logs

- [ ] Enregistrement des événements d'audit :
  - [ ] Setup initial, login/logout
  - [ ] CRUD sources, destinations, templates, règles
  - [ ] Mise à jour settings
  - [ ] Création/suppression/modification d'utilisateurs
  - [ ] Partage de configuration
- [ ] **Page `/audit`** — consultation de l'historique d'audit
  - [ ] Filtres : action, utilisateur, date
  - [ ] Pagination
  - [ ] Export CSV

---

## 🐳 Phase 12 — Docker & Déploiement

### Docker

- [ ] `Dockerfile` multi-stage (builder Rust + image finale minimale)
  - [x] Base `debian:bookworm-slim` ou `scratch` avec musl
  - [ ] COPY uniquement du binaire final
  - [x] User non-root dans le container
- [x] `.dockerignore` propre
- [x] `docker-compose.yml` pour run standalone
  - [x] Volume pour SQLite persistent
  - [x] Variables d'environnement via `.env`
- [x] `docker-compose.dev.yml` pour dev local
  - [x] Bind mount du workspace
  - [x] Cache `target` dedie
  - [x] Variables d'environnement via `.env`

### Traefik (prod)

- [x] `docker-compose.traefik.yml` avec Traefik
  - [x] Routing HTTP → HTTPS via Traefik
  - [x] TLS automatique via Let's Encrypt (ACME)
  - [x] Middleware : rate limiting, headers de sécurité
- [x] `deploy/traefik.yml` — configuration Traefik statique
- [x] `deploy/Caddyfile.example` — alternative Caddy
- [ ] Variables d'environnement documentées pour la prod
- [ ] Script de démarrage avec healthcheck

### Dev local

- [ ] `cargo watch` pour le rechargement automatique en dev
- [x] Makefile avec targets : `dev`, `build`, `migrate`, `test`, `lint`, `docker-build`
- [ ] `.env.example` avec toutes les variables commentées

---

## 🧪 Phase 13 — Tests & Qualité

- [x] Tests unitaires : normalisation des payloads (GitHub, GitLab, Gitea)
- [x] Tests unitaires : matching des règles de routage
- [x] Tests unitaires : génération des messages Discord
- [ ] Tests d'intégration : pipeline webhook complet (in-memory SQLite)
- [x] Tests de la validation de signature (HMAC)
- [ ] Tests des endpoints HTTP (`axum::test_helpers`)
- [ ] CI GitHub Actions :
  - [ ] `cargo fmt --check`
  - [ ] `cargo clippy -- -D warnings`
  - [ ] `cargo test`
  - [ ] Build Docker
- [ ] Coverage avec `cargo-tarpaulin`

---

## 📚 Phase 14 — Documentation

- [~] `README.md` complet :
  - [ ] Présentation du projet + screenshots UI
  - [x] Guide d'installation rapide (Docker)
  - [x] Guide de configuration
  - [x] Guide de contribution
- [x] `CHANGELOG.md`
- [x] Documentation des variables d'environnement
- [x] Documentation des templates Minijinja (variables disponibles par type d'événement)
- [x] Exemples de payloads par provider (GitHub, GitLab, Gitea)
- [x] Guide de déploiement avec Traefik

---

## 🚀 Backlog / Améliorations futures

- [ ] Retries automatiques Discord (exponential backoff)
- [ ] Destinations supplémentaires (Slack, Teams, ntfy, email)
- [ ] API admin JSON dédiée (REST complète)
- [ ] Webhooks sortants génériques (pas uniquement Discord)
- [ ] Notifications par email (SMTP)
- [ ] 2FA / TOTP pour les comptes admin
- [ ] Webhooks de test depuis l'interface (simulateur de payload)
- [ ] Statistiques avancées (graphiques d'activité par source/repo)
- [ ] Import/export complet de la configuration (JSON/YAML)
- [ ] Thèmes de couleur personnalisables par utilisateur
- [ ] Internationalisation (i18n) EN/FR
- [ ] Plugin system pour providers custom

---

_Dernière mise à jour : 2026-03-18 — documentation phase 14 ajoutee (`README`, `CHANGELOG`, docs config/templates/payloads/deploiement/contribution) + systeme roles/permissions et CRUD utilisateurs completes_
_Projet : DmxForge_
