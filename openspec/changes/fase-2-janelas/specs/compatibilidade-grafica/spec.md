# Spec Delta

## Purpose

Placas de vídeo e drivers suportados, com NVIDIA proprietário como requisito.

## ADDED Requirements

### Requirement: NVIDIA proprietário
O sistema MUST funcionar com o driver NVIDIA proprietário suportado (série 560 ou mais nova) com aceleração, sem artefatos e com suspensão.

#### Scenario: RTX com driver 580
- **WHEN** o sistema inicia numa RTX 3090 com driver 580
- **THEN** a sessão abre acelerada a 60 qps ou mais

### Requirement: AMD e Intel
O sistema MUST funcionar com placas AMD e Intel pelos drivers Mesa.

#### Scenario: Intel integrada
- **WHEN** o sistema inicia num notebook com Intel
- **THEN** a sessão abre acelerada

### Requirement: Modo de segurança
Se a aceleração falhar, o sistema MUST oferecer uma sessão por desenho em software.

#### Scenario: Driver quebrado
- **WHEN** o driver de vídeo não carrega
- **THEN** a recuperação oferece a sessão de segurança
