# Spec Delta

## Purpose

Levar o KalangoOS da imagem baixada a um computador pronto para uso.

## ADDED Requirements

### Requirement: Experimentar sem instalar
A imagem MUST abrir uma sessão completa sem escrever no disco.

#### Scenario: Experimentar
- **WHEN** o usuário escolhe Experimentar
- **THEN** o sistema funciona e o disco não é alterado

### Requirement: Modos de instalação
O instalador MUST oferecer disco inteiro, ao lado de outro sistema e particionamento manual.

#### Scenario: Dual boot
- **WHEN** o usuário instala ao lado do Ubuntu
- **THEN** os dois aparecem no menu de inicialização

### Requirement: Nada antes da confirmação
O instalador MUST NOT escrever no disco antes da confirmação final.

#### Scenario: Cancelar no passo 3
- **WHEN** o usuário cancela antes de confirmar
- **THEN** o disco fica intacto

### Requirement: Criptografia
O instalador MUST oferecer criptografia de disco, recomendada em notebooks.

#### Scenario: Notebook
- **WHEN** o instalador detecta bateria
- **THEN** a criptografia vem marcada

### Requirement: Primeiro uso
Após instalar, o primeiro uso MUST oferecer importar arquivos e favoritos e escolher aparência e como os agentes trabalham, sem repetir o que o instalador já perguntou.

#### Scenario: Idioma
- **WHEN** o idioma foi escolhido no instalador
- **THEN** o primeiro uso não pergunta de novo
