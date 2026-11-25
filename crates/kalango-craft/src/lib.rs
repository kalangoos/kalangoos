//! Os Apps criativos do projeto Craft (github.com/storytold) no KalangoOS.
//!
//! - `catalogo`: o nosso catálogo (`catalogo/apps-criativos.json`): os 17 apps, a chave do projeto fixada pela impressão
//!   digital e as versões que o KalangoOS bloqueia;
//! - `verificar`: o índice OCI do projeto, a assinatura OpenPGP de cada imagem (conferida com a chave fixada, pelo
//!   `gpgv`) e a cadeia assinatura → manifesto → configuração → camadas, cada uma pelo seu sha256;
//! - `instalar`: a imagem verificada entra num repositório local do Flatpak e é instalada dele (nada entra no sistema
//!   antes da verificação); atualizar guarda a versão anterior (o ponto de restauração do app); permissões por app;
//! - `niveis`: o que cada ferramenta MCP de um app pede quando um agente a usa (livre, confirmar, aprovar);
//! - `mcp`: o cliente MCP (stdio) que o kalangod usa para falar com o `<app>-cli mcp` de cada app;
//! - `ajustes`: o que a pessoa escolheu para cada app (atualização automática, rede, pastas).
//!
//! Nada aqui reempacota ou modifica os apps: eles chegam como o projeto os publica, com o nome e a marca deles.

pub mod ajustes;
pub mod catalogo;
pub mod instalar;
pub mod mcp;
pub mod niveis;
pub mod verificar;

pub use catalogo::{catalogo, App, Catalogo};
