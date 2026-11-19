# Spec Delta

## Purpose

Diálogos que todos os apps usam, desenhados e controlados pelo sistema.

## ADDED Requirements

### Requirement: Seletor de arquivos
Abrir e salvar arquivo MUST usar o diálogo do sistema, e o app MUST receber acesso só ao arquivo escolhido.

#### Scenario: App Flatpak
- **WHEN** um app Flatpak pede para abrir um arquivo
- **THEN** ele recebe acesso apenas ao arquivo escolhido

### Requirement: Permissões
Pedidos de câmera, microfone, tela e localização MUST mostrar quem pede e oferecer permitir enquanto em uso, só desta vez e não permitir.

#### Scenario: Microfone
- **WHEN** um app pede o microfone
- **THEN** o diálogo mostra o app e as três escolhas

### Requirement: Senha de administrador
O pedido de senha MUST dizer quem pede, o que muda e por quanto tempo vale.

#### Scenario: Instalar app
- **WHEN** a loja precisa instalar em /opt
- **THEN** o diálogo mostra a loja, o efeito e "só esta ação"

### Requirement: App travado
Quando um app não responder por 10 s, o sistema MUST oferecer esperar ou encerrar, e sugerir a saída menos destrutiva quando souber a causa.

#### Scenario: Aba pesada
- **WHEN** uma aba do navegador trava o app
- **THEN** o diálogo oferece fechar só a aba
