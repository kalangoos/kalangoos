# Tasks

## 1. Imagem e instalador

- [x] 1.1 Design do instalador (4 passos), inicialização, recuperação e primeiro uso
- [ ] 1.2 Receita reproduzível da ISO; dois builds geram o mesmo hash
- [ ] 1.3 Sessão de experimentar sem tocar o disco
- [ ] 1.4 Instalador com disco inteiro, ao lado de outro sistema e manual; teste em VM nos três modos
- [ ] 1.5 Criptografia LUKS2 com desbloqueio por TPM opcional
- [ ] 1.6 Primeiro uso com importação de arquivos e favoritos
- [ ] 1.7 Teste automático de instalação na CI (VM com UEFI e Secure Boot)

## 2. Atualização e restauração

- [x] 2.1 Ponto de restauração antes de toda atualização e listagem/volta pelo kalangod (`pontos.*`: btrfs ou cópia do /etc + lista de pacotes; validado na VM)
- [ ] 2.1b Volta em 1 clique na interface (app Configurações e tela de recuperação)
- [ ] 2.2 Volta automática quando a sessão gráfica não sobe após atualizar
- [ ] 2.3 Canais Estável e Beta; atualização de versão (0.x → 0.y) testada
- [ ] 2.4 Tela de recuperação: restaurar, modo seguro, reinstalar mantendo arquivos, verificar disco

## 3. Loja

- [ ] 3.1 Central de Apps com Flathub, agentes e modelos
- [ ] 3.2 Permissões mostradas e ajustáveis antes de instalar agentes
- [ ] 3.3 Modelos com aviso de cabe/não cabe na GPU

## 4. Contas e segurança

- [ ] 4.1 Usuários, administrador, convidado temporário
- [ ] 4.2 Impressão digital (fprintd) no login e no sudo
- [ ] 4.3 Inicialização com a marca (Plymouth) e tela de entrada própria
- [ ] 4.4 Repositório apt próprio assinado em kalangoos.com
