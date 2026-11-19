# Spec Delta

## Purpose

Atualizar com segurança e voltar atrás quando algo der errado.

## ADDED Requirements

### Requirement: Ponto antes de atualizar
Toda atualização do sistema MUST criar um ponto de restauração antes.

#### Scenario: Atualização
- **WHEN** uma atualização começa
- **THEN** um ponto de restauração aparece na lista

### Requirement: Volta automática
Se a sessão gráfica não iniciar após uma atualização, o sistema MUST voltar sozinho ao ponto anterior.

#### Scenario: Driver quebrado
- **WHEN** a atualização quebra o driver de vídeo
- **THEN** o próximo boot volta ao ponto anterior e avisa

### Requirement: Fora do horário de uso
Atualizações automáticas MUST acontecer fora do horário de uso aprendido e nunca reiniciar sem confirmação.

#### Scenario: Usuário trabalhando
- **WHEN** há atualização e o usuário está ativo
- **THEN** ela espera

### Requirement: Recuperação
Após duas falhas seguidas de inicialização, o sistema MUST abrir a recuperação com restaurar, modo seguro, reinstalar mantendo arquivos e verificar disco.

#### Scenario: Duas falhas
- **WHEN** o boot falha duas vezes
- **THEN** a tela de recuperação abre
