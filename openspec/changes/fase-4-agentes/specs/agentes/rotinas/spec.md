# Spec Delta

## Purpose

Agentes que trabalham sozinhos por horário ou gatilho.

## ADDED Requirements

### Requirement: Horário e gatilho
Rotinas MUST poder rodar por horário ou por evento (dispositivo conectado, rede, chegada de e-mail, troca de espaço).

#### Scenario: Pendrive
- **WHEN** um pendrive é conectado
- **THEN** a rotina de verificação roda

### Requirement: Mesmos limites
Rotinas MUST respeitar as permissões e limites do agente e MUST NOT furar o modo foco.

#### Scenario: Foco ativo
- **WHEN** uma rotina termina durante o modo foco
- **THEN** o aviso vai para o resumo
