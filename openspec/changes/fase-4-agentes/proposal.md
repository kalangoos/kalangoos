# Proposal

## Why

Agentes nativos são a razão de existir do KalangoOS. Para que operem o sistema inteiro com segurança, é preciso um
motor, um serviço privilegiado com regras do kernel, permissões, registro com desfazer e isolamento.

## What Changes

- `kalangod`: serviço do sistema que expõe as ferramentas de administração por MCP, em quatro níveis de poder.
- Motor de agentes na sessão do usuário: tarefas, modelos locais e de nuvem, memória, rotinas.
- Permissões (livre, perguntar, nunca), linha do tempo com desfazer, isolamento e auditoria pelo kernel.
- Conectores MCP de terceiros com teste isolado e cofre de segredos.
- App Agentes com o chat completo, modo voz e privacidade visível.

Fora do escopo: agentes de terceiros na loja (fase 5); administração remota.

## Capabilities

### New Capabilities
- `kalangod`: o MCP do sistema, que permite aos agentes nativos operar o sistema.
- `agentes/motor`: execução de tarefas, modelos e orçamento.
- `agentes/permissoes`: o que cada agente pode fazer sem perguntar.
- `agentes/linha-do-tempo`: registro de tudo o que os agentes fazem, com desfazer.
- `agentes/isolamento`: limites impostos pelo kernel a agentes e conectores.
- `agentes/conectores-mcp`: conectores MCP de terceiros.
- `agentes/memoria`: o que o sistema lembra do usuário.
- `agentes/rotinas`: tarefas agendadas e por gatilho.
- `agentes/chat`: a conversa com agentes e os conteúdos que ela exibe.
- `privacidade`: indicadores de uso de sensores e dados, e pausa geral.

### Modified Capabilities
Nenhuma.

## Impact

Crates `kalangod`, `kalango-agentes`, `kalango-ebpf`; app Agentes; parâmetro de inicialização `lsm=...,bpf`.
