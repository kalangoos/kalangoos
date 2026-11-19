# KalangoOS · visão de produto

## O que é

Uma distribuição Linux para computadores pessoais em que agentes de IA trabalham junto com o usuário: entendem
pedidos em linguagem natural, operam o sistema e os apps, e tudo o que fazem é visível, aprovado quando arriscado e
reversível.

## Para quem

| Público | O que procura | O que o KalangoOS entrega |
|---|---|---|
| Profissionais técnicos (desenvolvimento, dados, criação) | Um sistema rápido que automatize o trabalho repetitivo | Agentes com acesso ao terminal, aos arquivos e aos serviços, sob controle |
| Usuários que vêm do Windows ou do macOS | Um Linux bonito que não exija linha de comando | Interface própria, consistente, com ajuda que responde sem internet |
| Quem se preocupa com privacidade | IA sem mandar os dados para fora | Modelos locais por padrão; nuvem só com permissão e orçamento |

## Promessas do produto

1. **Agentes que fazem, não só conversam.** Eles administram o sistema de verdade, pelo `kalangod`.
2. **Nada irreversível sem você.** Ações arriscadas pedem aprovação; tudo entra na linha do tempo com desfazer.
3. **Privado por padrão.** Modelo local, memória criptografada, indicadores visíveis de microfone, câmera e tela.
4. **Leve e fluido.** Rust em tudo o que é nosso; animações com mola a 60 qps, inclusive em placas NVIDIA.
5. **Bonito e coerente.** Uma só linguagem visual em shell, apps e sistema; sem telas "de outro sistema".

## Diferenciais frente ao mercado

| | KalangoOS | Distros comuns | Windows / macOS |
|---|---|---|---|
| Agentes operando o sistema | Nativos, com níveis de poder e auditoria pelo kernel | Não | Assistentes limitados aos apps do fabricante |
| Desfazer o que a IA fez | Linha do tempo com desfazer | — | Não |
| IA local por padrão | Sim | Não | Parcial |
| Base | Ubuntu LTS, kernel atualizado | Variada | Proprietária |

## Fora do escopo da linha 1.x

Servidores, dispositivos móveis, administração remota de frotas, suporte comercial.

## Como medimos sucesso

- Instalação concluída sem ajuda por 9 de cada 10 pessoas em teste de usabilidade.
- Tarefas comuns de agente (organizar pasta, instalar app, corrigir erro) concluídas com aprovação em menos de 2 min.
- Nenhuma perda de dados causada por agente em testes de regressão.
- Metas de desempenho de `REQUISITOS.md` cumpridas em todas as versões.
